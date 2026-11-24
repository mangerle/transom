//! RDP 桌面视口 GPU Canvas 原生硬件直绘管线。
//!
//! 设计原理（Why）：
//! 1. 在 prepaint 阶段仅通过无重入的 `Rc<Cell<Bounds<Pixels>>>` 捕获物理视口矩形，
//!    彻底杜绝在渲染管线中调用 `entity.update(cx, ...)` 引发的 Double Lease 风险；
//! 2. 复用 [`ViewportTransform`] 统一完成 256x256 瓦片网格的等比居中映射、AABB 视口
//!    裁剪以及远端自定义光标位图的零延迟客户端硬件合成绘制。

use super::frame_cache::{RenderedCursor, TiledFrameCache, ViewportTransform};
use super::view::RdpView;
use gpui_kit::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use transom_rdp::RdpInput;
use transom_rdp::tiles::compute_tile_bounds;

/// 动态分辨率请求的最小合法尺寸阈值。
const MIN_DYNAMIC_WIDTH: u16 = 640;
const MIN_DYNAMIC_HEIGHT: u16 = 480;

impl RdpView {
    /// 渲染桌面瓦片集合与自定义光标的高性能 GPU 自绘画布。
    pub(crate) fn render_desktop_canvas(&self) -> impl IntoElement {
        let viewport_bounds_cell = Rc::clone(&self.viewport_bounds);
        let frame_cache = Rc::clone(&self.frame_cache);
        let cursor = self.cursor.clone();
        let last_mouse_pos = self.last_mouse_pos;
        let is_scaled = self.is_scaled;
        let dynamic_resolution = self.dynamic_resolution;
        let resize_sender = self.session_sender();

        canvas(
            move |bounds, _window, _cx| {
                let old_size = viewport_bounds_cell.get().size;
                viewport_bounds_cell.set(bounds);

                // 仅在启用动态分辨率、非等比缩放且视口尺寸实际改变时，通过独立通道请求重设分辨率
                if dynamic_resolution && !is_scaled && old_size != bounds.size {
                    let target_w = f32::from(bounds.size.width).round() as u16;
                    let target_h = f32::from(bounds.size.height).round() as u16;
                    if target_w >= MIN_DYNAMIC_WIDTH
                        && target_h >= MIN_DYNAMIC_HEIGHT
                        && let Some(ref sender) = resize_sender
                    {
                        let _ = sender.try_send(RdpInput::Resize {
                            width: target_w,
                            height: target_h,
                        });
                    }
                }
            },
            move |bounds, _, window, _cx| {
                paint_desktop_frame(
                    window,
                    bounds,
                    &frame_cache,
                    &cursor,
                    last_mouse_pos,
                    is_scaled,
                );
            },
        )
        .size_full()
    }
}

/// 在 GPUI Canvas 绘制阶段渲染所有可见瓦片及远端自定义光标。
fn paint_desktop_frame(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    frame_cache: &Rc<RefCell<TiledFrameCache>>,
    cursor: &RenderedCursor,
    last_mouse_pos: Option<(u16, u16)>,
    is_scaled: bool,
) {
    let cache = frame_cache.borrow();
    let Some(transform) = ViewportTransform::compute(bounds, cache.width, cache.height, is_scaled)
    else {
        return;
    };

    let cols = cache.cols as usize;
    let rows = cache.rows as usize;
    let view_r = bounds.origin.x + bounds.size.width;
    let view_b = bounds.origin.y + bounds.size.height;

    // 1. 批量绘制可见增量瓦片（AABB 视口裁剪）
    for r in 0..rows {
        for c in 0..cols {
            let idx = r * cols + c;
            if let Some(Some(tile_img)) = cache.tiles.get(idx) {
                let (tile_x, tile_y, tile_w, tile_h) =
                    compute_tile_bounds(cache.width, cache.height, c as u32, r as u32);
                let tile_bounds = transform.map_desktop_rect(
                    bounds.origin,
                    tile_x as f32,
                    tile_y as f32,
                    tile_w as f32,
                    tile_h as f32,
                );

                if tile_bounds.origin.x + tile_bounds.size.width <= bounds.origin.x
                    || tile_bounds.origin.x >= view_r
                    || tile_bounds.origin.y + tile_bounds.size.height <= bounds.origin.y
                    || tile_bounds.origin.y >= view_b
                {
                    continue;
                }

                let _ = window.paint_image(
                    bounds,
                    tile_bounds,
                    Corners::default(),
                    tile_img.clone(),
                    0,
                    false,
                );
            }
        }
    }

    // 2. 客户端硬件合成绘制远端自定义位图光标（零网络延迟）
    if let (
        RenderedCursor::Custom {
            width,
            height,
            hot_x,
            hot_y,
            image,
        },
        Some((mx, my)),
    ) = (cursor, last_mouse_pos)
    {
        let cursor_bounds = transform.map_desktop_rect(
            bounds.origin,
            f32::from(mx) - f32::from(*hot_x),
            f32::from(my) - f32::from(*hot_y),
            f32::from(*width),
            f32::from(*height),
        );
        let _ = window.paint_image(
            bounds,
            cursor_bounds,
            Corners::default(),
            image.clone(),
            0,
            false,
        );
    }
}

/// 便捷构造用于 prepaint 的无重入视口包围盒容器。
pub(crate) fn new_viewport_bounds_cell() -> Rc<Cell<Bounds<Pixels>>> {
    Rc::new(Cell::new(Bounds::default()))
}
