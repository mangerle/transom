//! RDP 桌面视口纹理缓存、光标纹理与统一坐标变换。
//!
//! 设计原理（Why）：
//! 1. 将 GPU 纹理管理（`Arc<RenderImage>`）收敛在 UI 层，使底层协议库 `transom-rdp`
//!    与 GUI 框架彻底解耦；
//! 2. [`TiledFrameCache`] 仅按增量补丁 [`FrameUpdate`] 原地替换受损瓦片槽位，干净瓦片
//!    复用已有 `Arc<RenderImage>`，彻底消除每帧克隆整张瓦片表的开销；
//! 3. [`ViewportTransform`] 作为渲染与鼠标坐标反算的唯一真源，杜绝两处缩放公式漂移。

use gpui_kit::{Bounds, Pixels, Point, RenderImage, Size, Window, px};
use image::{Frame, RgbaImage};
use std::sync::Arc;
use transom_rdp::{CursorUpdate, FrameUpdate};

/// 待回收 GPU 纹理队列上限（防异常状态下内存无界积压）。
const MAX_STALE_IMAGES: usize = 512;

/// UI 侧常驻的桌面瓦片 GPU 纹理缓存。
///
/// 设计原理（Why）：GPUI 的 `window.paint_image` 会以 `ImageId` 将 `RenderImage`
/// 驻留在窗口级 GPU Sprite Atlas 中；仅覆盖并 `Drop` Rust 侧的 `Arc<RenderImage>`
/// 不会自动从图集卸载显存。因此每次替换或清空瓦片时，必须将淘汰的纹理句柄收集到
/// `stale_images`，在持有 `&mut Window` 的渲染/绘制周期中显式调用 `window.drop_image`
/// 确定性释放显存。
#[derive(Default)]
pub(crate) struct TiledFrameCache {
    pub(crate) width: u16,
    pub(crate) height: u16,
    pub(crate) cols: u32,
    pub(crate) rows: u32,
    pub(crate) tiles: Vec<Option<Arc<RenderImage>>>,
    stale_images: Vec<Arc<RenderImage>>,
}

impl TiledFrameCache {
    /// 应用来自后台会话的增量瓦片补丁，仅对变更瓦片构造新纹理并原地覆写，
    /// 同时将淘汰的旧纹理移入待回收队列。
    pub(crate) fn apply_update(&mut self, update: FrameUpdate) {
        let total = (update.cols as usize).saturating_mul(update.rows as usize);
        if update.full_reset
            || self.width != update.width
            || self.height != update.height
            || self.tiles.len() != total
        {
            self.retire_all_tiles();
            self.width = update.width;
            self.height = update.height;
            self.cols = update.cols;
            self.rows = update.rows;
            self.tiles = vec![None; total];
        }

        let cols = self.cols as usize;
        for patch in update.patches {
            let idx = (patch.row as usize)
                .saturating_mul(cols)
                .saturating_add(patch.col as usize);
            if idx < self.tiles.len()
                && let Some(rgba) = RgbaImage::from_raw(patch.width, patch.height, patch.rgba)
            {
                let img = Arc::new(RenderImage::new(vec![Frame::new(rgba)]));
                if let Some(old) = self.tiles[idx].replace(img) {
                    self.retire_image(old);
                }
            }
        }
    }

    /// 将一张已淘汰的 GPU 纹理（瓦片或自定义光标）登记到待回收队列。
    pub(crate) fn retire_image(&mut self, img: Arc<RenderImage>) {
        if self.stale_images.len() < MAX_STALE_IMAGES {
            self.stale_images.push(img);
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

    /// 清空已缓存瓦片并将旧纹理移入待回收队列（如分辨率变更重激活或断开期间）。
    pub(crate) fn clear(&mut self) {
        self.retire_all_tiles();
        self.width = 0;
        self.height = 0;
        self.cols = 0;
        self.rows = 0;
        self.tiles.clear();
    }

    /// 返回当前待回收纹理数量（主要用于单元测试断言）。
    #[cfg(test)]
    pub(crate) fn stale_count(&self) -> usize {
        self.stale_images.len()
    }

    fn retire_all_tiles(&mut self) {
        for slot in &mut self.tiles {
            if let Some(old) = slot.take()
                && self.stale_images.len() < MAX_STALE_IMAGES
            {
                self.stale_images.push(old);
            }
        }
    }
}

/// UI 侧已解码的光标渲染状态。
#[derive(Clone)]
pub(crate) enum RenderedCursor {
    Default,
    Hidden,
    Custom {
        width: u16,
        height: u16,
        hot_x: u16,
        hot_y: u16,
        image: Arc<RenderImage>,
    },
}

impl From<CursorUpdate> for RenderedCursor {
    fn from(update: CursorUpdate) -> Self {
        match update {
            CursorUpdate::Default => Self::Default,
            CursorUpdate::Hidden => Self::Hidden,
            CursorUpdate::Custom {
                width,
                height,
                hot_x,
                hot_y,
                rgba,
            } => {
                if let Some(img) =
                    RgbaImage::from_raw(u32::from(width), u32::from(height), rgba.to_vec())
                {
                    Self::Custom {
                        width,
                        height,
                        hot_x,
                        hot_y,
                        image: Arc::new(RenderImage::new(vec![Frame::new(img)])),
                    }
                } else {
                    Self::Default
                }
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
}

impl ViewportTransform {
    /// 根据视口物理矩形、远端桌面尺寸、缩放模式与 1:1 模式平移偏移计算映射变换。
    pub(crate) fn compute(
        viewport_bounds: Bounds<Pixels>,
        desktop_width: u16,
        desktop_height: u16,
        scale_to_fit: bool,
        pan_offset: (f32, f32),
    ) -> Option<Self> {
        if desktop_width == 0 || desktop_height == 0 {
            return None;
        }
        let view_w = f32::from(viewport_bounds.size.width);
        let view_h = f32::from(viewport_bounds.size.height);
        if view_w <= 0.0 || view_h <= 0.0 {
            return None;
        }

        let desk_w = f32::from(desktop_width);
        let desk_h = f32::from(desktop_height);
        if scale_to_fit {
            let scale = (view_w / desk_w).min(view_h / desk_h);
            if scale <= 0.0 {
                return None;
            }
            let offset_x = (view_w - desk_w * scale) / 2.0;
            let offset_y = (view_h - desk_h * scale) / 2.0;
            Some(Self {
                scale,
                offset_x,
                offset_y,
            })
        } else {
            let min_x = (view_w - desk_w).min(0.0);
            let min_y = (view_h - desk_h).min(0.0);
            Some(Self {
                scale: 1.0,
                offset_x: pan_offset.0.clamp(min_x, 0.0),
                offset_y: pan_offset.1.clamp(min_y, 0.0),
            })
        }
    }

    /// 将远端桌面局部矩形映射为视口绝对绘制包围盒。
    pub(crate) fn map_desktop_rect(
        &self,
        viewport_origin: Point<Pixels>,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    ) -> Bounds<Pixels> {
        Bounds {
            origin: viewport_origin
                + Point::new(
                    px(self.offset_x + x * self.scale),
                    px(self.offset_y + y * self.scale),
                ),
            size: Size {
                width: px(w * self.scale),
                height: px(h * self.scale),
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
