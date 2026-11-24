//! RDP 桌面视口纹理缓存、光标纹理与统一坐标变换。
//!
//! 设计原理（Why）：
//! 1. 将 GPU 纹理管理（`Arc<RenderImage>`）收敛在 UI 层，使底层协议库 `transom-rdp`
//!    与 GUI 框架彻底解耦；
//! 2. **延迟纹理上传（Deferred Texture Upload）**：后台推送的 [`FrameUpdate`] 先在纯 CPU
//!    待上传槽位中按 `(col, row)` 覆盖合并，仅在窗口实际执行绘制（`paint`）时才构造
//!    `RenderImage` 并立即 `window.drop_image` 释放被淘汰的旧纹理，彻底消除后台标签页
//!    不可见时的 GPU 纹理创建开销与显存泄漏；
//! 3. [`ViewportTransform`] 作为渲染与鼠标坐标反算的唯一真源，杜绝两处缩放公式漂移。

use gpui_kit::{Bounds, Pixels, Point, RenderImage, Size, Window, px};
use image::{Frame, RgbaImage};
use std::sync::Arc;
use transom_rdp::{FrameUpdate, TileBufferPool, TilePatch};

/// UI 侧常驻的桌面瓦片 GPU 纹理缓存。
#[derive(Default)]
pub(crate) struct TiledFrameCache {
    pub(crate) width: u16,
    pub(crate) height: u16,
    pub(crate) cols: u32,
    pub(crate) rows: u32,
    pub(crate) tiles: Vec<Option<Arc<RenderImage>>>,
    pending_reset: bool,
    pending_size: Option<(u16, u16, u32, u32)>,
    pending_is_bgra: bool,
    pending_patches: Vec<Option<TilePatch>>,
    stale_images: Vec<Arc<RenderImage>>,
    buffer_pool: Option<TileBufferPool>,
}

impl TiledFrameCache {
    /// 绑定与后端共享的像素缓冲池，使被覆盖/清理的未上传补丁能归还复用。
    pub(crate) fn set_buffer_pool(&mut self, pool: TileBufferPool) {
        self.buffer_pool = Some(pool);
    }

    fn recycle_patch(&self, patch: TilePatch) {
        if let Some(pool) = &self.buffer_pool {
            pool.release(patch.rgba);
        }
    }

    fn recycle_all_pending(&mut self) {
        if let Some(pool) = &self.buffer_pool {
            for slot in &mut self.pending_patches {
                if let Some(old) = slot.take() {
                    pool.release(old.rgba);
                }
            }
        } else {
            self.pending_patches.clear();
        }
    }

    /// 接收后台会话的增量瓦片补丁，在纯 CPU 待上传缓冲中按瓦片槽位就地合并。
    pub(crate) fn apply_update(&mut self, update: FrameUpdate) {
        let total = (update.cols as usize).saturating_mul(update.rows as usize);
        let size_changed = self.pending_size.map_or_else(
            || {
                self.width != update.width
                    || self.height != update.height
                    || self.tiles.len() != total
            },
            |(pw, ph, pc, pr)| {
                pw != update.width || ph != update.height || pc != update.cols || pr != update.rows
            },
        );

        if update.full_reset || size_changed {
            self.recycle_all_pending();
            self.pending_reset = true;
            self.pending_size = Some((update.width, update.height, update.cols, update.rows));
            self.pending_patches.resize_with(total, || None);
        } else if self.pending_patches.len() != total {
            self.pending_patches.resize_with(total, || None);
        }

        let cols = self.pending_size.map_or(self.cols, |(_, _, c, _)| c) as usize;
        self.pending_is_bgra = update.is_bgra;
        for patch in update.patches {
            let idx = (patch.row as usize)
                .saturating_mul(cols)
                .saturating_add(patch.col as usize);
            if let Some(slot) = self.pending_patches.get_mut(idx) {
                if let Some(replaced) = slot.replace(patch) {
                    self.recycle_patch(replaced);
                }
            } else {
                self.recycle_patch(patch);
            }
        }
    }

    /// 将待上传的纯像素补丁提交为 GPU 纹理，并把被替换的旧纹理登记到回收队列。
    pub(crate) fn commit_pending(&mut self) {
        if self.pending_reset {
            self.retire_all_tiles();
            if let Some((w, h, cols, rows)) = self.pending_size.take() {
                self.width = w;
                self.height = h;
                self.cols = cols;
                self.rows = rows;
                let total = (cols as usize).saturating_mul(rows as usize);
                self.tiles = vec![None; total];
            }
            self.pending_reset = false;
        }

        for slot in &mut self.pending_patches {
            let Some(mut patch) = slot.take() else {
                continue;
            };
            let idx = (patch.row as usize)
                .saturating_mul(self.cols as usize)
                .saturating_add(patch.col as usize);
            if idx < self.tiles.len() {
                // 仅在输入缓冲非 BGRA8 时原地交换 R/B 通道；BgrA32/BgrX32 直出零转换
                if !self.pending_is_bgra {
                    for px in patch.rgba.as_chunks_mut::<4>().0 {
                        px.swap(0, 2);
                    }
                }
                if let Some(rgba) = RgbaImage::from_raw(patch.width, patch.height, patch.rgba) {
                    let img = Arc::new(RenderImage::new(vec![Frame::new(rgba)]));
                    if let Some(old) = self.tiles[idx].replace(img) {
                        self.stale_images.push(old);
                    }
                }
            }
        }
    }

    /// 在持有窗口上下文时，确定性释放全部已淘汰的 GPU Sprite Atlas 纹理。
    pub(crate) fn drain_stale_images(&mut self, window: &mut Window) {
        for img in self.stale_images.drain(..) {
            if let Err(err) = window.drop_image(img) {
                log::debug!("卸载旧 RDP GPU 纹理失败，忽略继续: {err}");
            }
        }
    }

    /// 清空已缓存瓦片与待上传补丁，并将现役纹理移入待回收队列。
    pub(crate) fn clear(&mut self) {
        self.retire_all_tiles();
        self.width = 0;
        self.height = 0;
        self.cols = 0;
        self.rows = 0;
        self.tiles.clear();
        self.pending_reset = false;
        self.pending_size = None;
        self.recycle_all_pending();
        self.pending_patches.clear();
    }

    /// 返回当前待回收纹理数量（主要用于单元测试断言）。
    #[cfg(test)]
    pub(crate) fn stale_count(&self) -> usize {
        self.stale_images.len()
    }

    fn retire_all_tiles(&mut self) {
        for slot in &mut self.tiles {
            if let Some(old) = slot.take() {
                self.stale_images.push(old);
            }
        }
    }
}

/// 远端桌面坐标系与本地视口坐标系之间的等比居中映射参数（渲染与输入共享唯一真源）。
#[derive(Debug, Clone, Copy)]
pub(crate) struct ViewportTransform {
    pub(crate) scale: f32,
    pub(crate) offset_x: f32,
    pub(crate) offset_y: f32,
    pub(crate) scale_factor: f32,
}

#[inline]
fn snap_to_physical_pixel(logical: f32, scale_factor: f32) -> f32 {
    (logical * scale_factor).round() / scale_factor
}

impl ViewportTransform {
    /// 根据视口物理矩形、远端桌面尺寸、缩放模式、1:1 模式平移偏移与屏幕 DPI 缩放计算映射变换。
    pub(crate) fn compute(
        viewport_bounds: Bounds<Pixels>,
        desktop_width: u16,
        desktop_height: u16,
        scale_to_fit: bool,
        pan_offset: (f32, f32),
        scale_factor: f32,
    ) -> Option<Self> {
        if desktop_width == 0 || desktop_height == 0 {
            return None;
        }
        let view_w = f32::from(viewport_bounds.size.width);
        let view_h = f32::from(viewport_bounds.size.height);
        if view_w <= 0.0 || view_h <= 0.0 {
            return None;
        }

        let sf = scale_factor.max(1.0);
        let desk_w = f32::from(desktop_width);
        let desk_h = f32::from(desktop_height);
        if scale_to_fit {
            let phys_w = view_w * sf;
            let phys_h = view_h * sf;
            // 若远端像素分辨率与本地物理视口几乎一致（<=2px 容差），锁定 1:1 物理像素点对点渲染
            let scale = if (phys_w - desk_w).abs() <= 2.0 && (phys_h - desk_h).abs() <= 2.0 {
                1.0 / sf
            } else {
                (view_w / desk_w).min(view_h / desk_h)
            };
            if scale <= 0.0 {
                return None;
            }
            let offset_x = snap_to_physical_pixel((view_w - desk_w * scale) / 2.0, sf);
            let offset_y = snap_to_physical_pixel((view_h - desk_h * scale) / 2.0, sf);
            Some(Self {
                scale,
                offset_x,
                offset_y,
                scale_factor: sf,
            })
        } else {
            let scale = 1.0 / sf;
            let min_x = (view_w - desk_w * scale).min(0.0);
            let min_y = (view_h - desk_h * scale).min(0.0);
            Some(Self {
                scale,
                offset_x: snap_to_physical_pixel(pan_offset.0.clamp(min_x, 0.0), sf),
                offset_y: snap_to_physical_pixel(pan_offset.1.clamp(min_y, 0.0), sf),
                scale_factor: sf,
            })
        }
    }

    /// 将远端桌面局部矩形映射为视口绝对绘制包围盒（四边对齐物理像素以消除模糊与接缝）。
    pub(crate) fn map_desktop_rect(
        &self,
        viewport_origin: Point<Pixels>,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    ) -> Bounds<Pixels> {
        let left = snap_to_physical_pixel(self.offset_x + x * self.scale, self.scale_factor);
        let top = snap_to_physical_pixel(self.offset_y + y * self.scale, self.scale_factor);
        let right = snap_to_physical_pixel(
            self.offset_x + (x + w) * self.scale,
            self.scale_factor,
        );
        let bottom = snap_to_physical_pixel(
            self.offset_y + (y + h) * self.scale,
            self.scale_factor,
        );
        Bounds {
            origin: viewport_origin + Point::new(px(left), px(top)),
            size: Size {
                width: px((right - left).max(0.0)),
                height: px((bottom - top).max(0.0)),
            },
        }
    }

    /// 将视口内的鼠标物理坐标反算为远端桌面像素坐标；落在黑边或视口外时返回 `None`。
    pub(crate) fn unmap_point(
        &self,
        viewport_bounds: Bounds<Pixels>,
        cursor_pos: Point<Pixels>,
        desktop_width: u16,
        desktop_height: u16,
    ) -> Option<(u16, u16)> {
        if !viewport_bounds.contains(&cursor_pos) || self.scale <= 0.0 {
            return None;
        }
        let rel_x = f32::from(cursor_pos.x - viewport_bounds.origin.x);
        let rel_y = f32::from(cursor_pos.y - viewport_bounds.origin.y);
        let desk_x = (rel_x - self.offset_x) / self.scale;
        let desk_y = (rel_y - self.offset_y) / self.scale;

        if desk_x < 0.0
            || desk_y < 0.0
            || desk_x >= f32::from(desktop_width)
            || desk_y >= f32::from(desktop_height)
        {
            return None;
        }

        let px_x = (desk_x.round() as u16).min(desktop_width.saturating_sub(1));
        let px_y = (desk_y.round() as u16).min(desktop_height.saturating_sub(1));
        Some((px_x, px_y))
    }
}
