//! RDP 桌面视口 GPU Canvas 原生硬件直绘管线。
//!
//! 设计原理（Why）：
//! 1. 在 prepaint 阶段仅通过无重入的 `Rc<Cell<Bounds<Pixels>>>` 捕获物理视口矩形，
//!    彻底杜绝在渲染管线中调用 `entity.update(cx, ...)` 引发的 Double Lease 风险；
//! 2. 复用 [`ViewportTransform`] 统一完成 256x256 瓦片网格的等比居中/1:1 平移映射、
//!    AABB 视口裁剪以及过期 GPU 纹理的确定性释放（`window.drop_image`）。

use super::frame_cache::{TiledFrameCache, ViewportTransform};
use super::view::RdpView;
use gpui_kit::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use transom_rdp::tiles::{TILE_SIZE, compute_tile_bounds};
use transom_rdp::{MIN_DESKTOP_HEIGHT, MIN_DESKTOP_WIDTH, RdpInput};

/// 视口矩形与动态分辨率请求协同状态（收敛 `prepaint` 阶段所需的无重入共享状态）。
#[derive(Clone, Copy, Default)]
pub(crate) struct ViewportSyncState {
    bounds: Bounds<Pixels>,
    scale_factor: f32,
    last_sent_resize: Option<(u16, u16, u32)>,
}

/// 无重入视口状态句柄。
#[derive(Clone, Default)]
pub(crate) struct ViewportSyncHandle(Rc<Cell<ViewportSyncState>>);

impl ViewportSyncHandle {
    pub(crate) fn bounds(&self) -> Bounds<Pixels> {
        self.0.get().bounds
    }

    pub(crate) fn scale_factor(&self) -> f32 {
        self.0.get().scale_factor.max(1.0)
    }

    pub(crate) fn scale_factor_percent(&self) -> u32 {
        let scale = self.0.get().scale_factor.max(1.0);
        ((scale * 100.0).round() as u32).clamp(100, 500)
    }

    pub(crate) fn set_scale_factor(&self, scale_factor: f32) {
        let mut state = self.0.get();
        state.scale_factor = scale_factor.max(1.0);
        self.0.set(state);
    }

    pub(crate) fn set_last_sent_resize(&self, size: Option<(u16, u16)>) {
        let mut state = self.0.get();
        let scale = ((state.scale_factor.max(1.0) * 100.0).round() as u32).clamp(100, 500);
        state.last_sent_resize = size.map(|(w, h)| (w, h, scale));
        self.0.set(state);
    }

    fn handle_prepaint(
        &self,
        bounds: Bounds<Pixels>,
        scale_factor: f32,
        dynamic_resolution: bool,
        resize_sender: Option<&tokio::sync::mpsc::Sender<RdpInput>>,
    ) {
        let mut state = self.0.get();
        let old_size = state.bounds.size;
        state.bounds = bounds;
        let scale = scale_factor.max(1.0);
        state.scale_factor = scale;

        if dynamic_resolution && f32::from(old_size.width) > 0.0 && f32::from(old_size.height) > 0.0
        {
            let target_w = (f32::from(bounds.size.width) * scale).round() as u16;
            let target_h = (f32::from(bounds.size.height) * scale).round() as u16;
            let scale_percent = ((scale * 100.0).round() as u32).clamp(100, 500);
            // 跳过首次从 0x0 布局初始化的抖动，且通道满时不覆写已发送尺寸以便下次重试
            if target_w >= MIN_DESKTOP_WIDTH
                && target_h >= MIN_DESKTOP_HEIGHT
                && state.last_sent_resize != Some((target_w, target_h, scale_percent))
                && let Some(sender) = resize_sender
                && sender
                    .try_send(RdpInput::Resize {
                        width: target_w,
                        height: target_h,
                        scale_factor_percent: scale_percent,
                    })
                    .is_ok()
            {
                state.last_sent_resize = Some((target_w, target_h, scale_percent));
            }
        }
        self.0.set(state);
    }
}

impl RdpView {
    /// 渲染桌面瓦片集合的高性能 GPU 自绘画布。
    pub(crate) fn render_desktop_canvas(&self) -> impl IntoElement {
        let viewport_sync = self.viewport_sync.clone();
        let frame_cache = Rc::clone(&self.frame_cache);
        let is_scaled = self.is_scaled;
        let pan_offset = self.input.pan_offset;
        let dynamic_resolution = self.dynamic_resolution;
        let resize_sender = self.session_sender();

        canvas(
            move |bounds, window, _cx| {
                viewport_sync.handle_prepaint(
                    bounds,
                    window.scale_factor(),
                    dynamic_resolution,
                    resize_sender.as_ref(),
                );
            },
            move |bounds, _, window, _cx| {
                paint_desktop_frame(window, bounds, &frame_cache, is_scaled, pan_offset);
            },
        )
        .size_full()
    }
}

/// 在 GPUI Canvas 绘制阶段释放淘汰纹理并渲染所有可见瓦片。
fn paint_desktop_frame(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    frame_cache: &Rc<RefCell<TiledFrameCache>>,
    is_scaled: bool,
    pan_offset: (f32, f32),
) {
    {
        let mut cache = frame_cache.borrow_mut();
        cache.commit_pending();
        cache.drain_stale_images(window);
    }
    let cache = frame_cache.borrow();
    let Some(transform) = ViewportTransform::compute(
        bounds,
        cache.width,
        cache.height,
        is_scaled,
        pan_offset,
        window.scale_factor(),
    ) else {
        return;
    };

    paint_visible_tiles(window, bounds, &cache, &transform);
}

/// 批量绘制落入视口可见区间内的增量瓦片（直接反算行列范围，避免全网格遍历）。
fn paint_visible_tiles(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    cache: &TiledFrameCache,
    transform: &ViewportTransform,
) {
    if cache.cols == 0 || cache.rows == 0 || transform.scale <= 0.0 {
        return;
    }

    let view_w = f32::from(bounds.size.width);
    let view_h = f32::from(bounds.size.height);
    let vis_left = ((0.0 - transform.offset_x) / transform.scale)
        .max(0.0)
        .floor() as u32;
    let vis_top = ((0.0 - transform.offset_y) / transform.scale)
        .max(0.0)
        .floor() as u32;
    let vis_right = ((view_w - transform.offset_x) / transform.scale)
        .min(f32::from(cache.width))
        .ceil() as u32;
    let vis_bottom = ((view_h - transform.offset_y) / transform.scale)
        .min(f32::from(cache.height))
        .ceil() as u32;

    if vis_left >= vis_right || vis_top >= vis_bottom {
        return;
    }

    let col_start = (vis_left / TILE_SIZE).min(cache.cols - 1);
    let col_end = ((vis_right - 1) / TILE_SIZE).min(cache.cols - 1);
    let row_start = (vis_top / TILE_SIZE).min(cache.rows - 1);
    let row_end = ((vis_bottom - 1) / TILE_SIZE).min(cache.rows - 1);
    let cols = cache.cols as usize;

    for r in row_start..=row_end {
        for c in col_start..=col_end {
            let idx = (r as usize) * cols + (c as usize);
            let Some(Some(tile_img)) = cache.tiles.get(idx) else {
                continue;
            };
            let (tile_x, tile_y, tile_w, tile_h) =
                compute_tile_bounds(cache.width, cache.height, c, r);
            let tile_bounds = transform.map_desktop_rect(
                bounds.origin,
                tile_x as f32,
                tile_y as f32,
                tile_w as f32,
                tile_h as f32,
            );

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
