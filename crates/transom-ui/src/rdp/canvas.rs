//! RDP 桌面视口 GPU Canvas 原生硬件直绘管线。
//!
//! 设计原理（Why）：
//! 1. 在 prepaint 阶段仅通过无重入的 `Rc<Cell<Bounds<Pixels>>>` 捕获物理视口矩形，
//!    彻底杜绝在渲染管线中调用 `entity.update(cx, ...)` 引发的 Double Lease 风险；
//! 2. 复用 [`ViewportTransform`] 统一完成 256x256 瓦片网格的等比居中/1:1 平移映射、
//!    AABB 视口裁剪以及过期 GPU 纹理的确定性释放（`window.drop_image`）。

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
        let last_mouse_pos = Rc::clone(&self.input.last_mouse_pos);
        let is_scaled = self.is_scaled;
        let pan_offset = self.input.pan_offset;
        let dynamic_resolution = self.dynamic_resolution;
        let resize_sender = self.session_sender();
        let last_sent_resize = Rc::clone(&self.last_sent_resize);

        canvas(
            move |bounds, window, _cx| {
                let old_size = viewport_bounds_cell.get().size;
                viewport_bounds_cell.set(bounds);

                if dynamic_resolution && f32::from(old_size.width) > 0.0 && f32::from(old_size.height) > 0.0
                {
                    let scale = window.scale_factor().max(1.0);
                    let target_w = (f32::from(bounds.size.width) * scale).round() as u16;
                    let target_h = (f32::from(bounds.size.height) * scale).round() as u16;
                    // 跳过首次从 0x0 布局初始化的抖动，且通道满时不覆写已发送尺寸以便下次重试
                    if target_w >= MIN_DYNAMIC_WIDTH
                        && target_h >= MIN_DYNAMIC_HEIGHT
                        && last_sent_resize.get() != Some((target_w, target_h))
                        && let Some(ref sender) = resize_sender
                        && sender
                            .try_send(RdpInput::Resize {
                                width: target_w,
                                height: target_h,
                            })
                            .is_ok()
                    {
                        last_sent_resize.set(Some((target_w, target_h)));
                    }
                }
            },
            move |bounds, _, window, _cx| {
                paint_desktop_frame(
                    window,
                    bounds,
                    &frame_cache,
                    &cursor,
                    last_mouse_pos.get(),
                    is_scaled,
                    pan_offset,
                );
            },
        )
        .size_full()
    }
}

/// 在 GPUI Canvas 绘制阶段释放淘汰纹理并渲染所有可见瓦片与光标。
fn paint_desktop_frame(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    frame_cache: &Rc<RefCell<TiledFrameCache>>,
    cursor: &RenderedCursor,
    last_mouse_pos: Option<(u16, u16)>,
    is_scaled: bool,
    pan_offset: (f32, f32),
) {
    {
        let mut cache = frame_cache.borrow_mut();
        cache.commit_pending();
        cache.drain_stale_images(window);
    }
    let cache = frame_cache.borrow();
    let Some(transform) =
        ViewportTransform::compute(bounds, cache.width, cache.height, is_scaled, pan_offset)
    else {
        return;
    };

    paint_visible_tiles(window, bounds, &cache, &transform);
    paint_custom_cursor(window, bounds, cursor, last_mouse_pos, &transform);
}

/// 批量绘制落入视口 AABB 范围内的增量瓦片。
fn paint_visible_tiles(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    cache: &TiledFrameCache,
    transform: &ViewportTransform,
) {
    let cols = cache.cols as usize;
    let rows = cache.rows as usize;
    let view_r = bounds.origin.x + bounds.size.width;
    let view_b = bounds.origin.y + bounds.size.height;

    for r in 0..rows {
        for c in 0..cols {
            let idx = r * cols + c;
            let Some(Some(tile_img)) = cache.tiles.get(idx) else {
                continue;
            };
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

/// 若启用了自定义位图光标则执行客户端硬件合成绘制。
fn paint_custom_cursor(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    cursor: &RenderedCursor,
    last_mouse_pos: Option<(u16, u16)>,
    transform: &ViewportTransform,
) {
    let (
        RenderedCursor::Custom {
            width,
            height,
            hot_x,
            hot_y,
            image,
        },
        Some((mx, my)),
    ) = (cursor, last_mouse_pos)
    else {
        return;
    };

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

/// 便捷构造用于 prepaint 的无重入视口包围盒容器。
pub(crate) fn new_viewport_bounds_cell() -> Rc<Cell<Bounds<Pixels>>> {
    Rc::new(Cell::new(Bounds::default()))
}
