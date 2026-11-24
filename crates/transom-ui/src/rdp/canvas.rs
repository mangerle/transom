//! RDP 桌面视口 GPU Canvas 原生硬件直绘管线。
//!
//! 设计原理（Why）：将 GPU Canvas 渲染逻辑从主视图结构中独立拆分，遵循关注点分离原则。
//! 负责桌面 256x256 瓦片网格的缩放变换、视口 AABB 裁剪（Frustum Culling）以及
//! 远端自定义光标的无延迟客户端硬件合成绘制。

use gpui_kit::*;
use transom_rdp::{CursorUpdate, RdpInput};

use super::view::RdpView;

impl RdpView {
    /// 渲染桌面瓦片集合的高性能 GPU 自绘画布（含视口 AABB 裁剪与客户端光标渲染）。
    pub(crate) fn render_desktop_canvas(&self, cx: &Context<Self>) -> impl IntoElement {
        let view_entity = cx.entity().clone();
        let frame = self.frame.clone();
        let is_scaled = self.is_scaled;
        let cursor = self.cursor.clone();
        let last_mouse_pos = self.last_mouse_pos;

        canvas(
            move |bounds, _window, cx| {
                view_entity.update(cx, |this, _| {
                    let old_size = this.viewport_bounds.size;
                    this.viewport_bounds = bounds;
                    // 若未锁定等比缩放且视口发生有效改变，联动请求动态调整分辨率
                    if !this.is_scaled && old_size != bounds.size {
                        let target_w = f32::from(bounds.size.width) as u16;
                        let target_h = f32::from(bounds.size.height) as u16;
                        if target_w >= 640 && target_h >= 480 {
                            this.send_input(RdpInput::Resize {
                                width: target_w,
                                height: target_h,
                            });
                        }
                    }
                });
            },
            move |bounds, _, window, _cx| {
                let Some(frame) = &frame else {
                    return;
                };

                let cols = frame.cols as usize;
                let rows = frame.rows as usize;
                let desktop_width = frame.width;
                let desktop_height = frame.height;

                if desktop_width == 0 || desktop_height == 0 {
                    return;
                }

                let (scale_x, scale_y) = if is_scaled {
                    (
                        f32::from(bounds.size.width) / f32::from(desktop_width),
                        f32::from(bounds.size.height) / f32::from(desktop_height),
                    )
                } else {
                    (1.0, 1.0)
                };

                let view_r = bounds.origin.x + bounds.size.width;
                let view_b = bounds.origin.y + bounds.size.height;

                // 1. 批量绘制增量可见瓦片（执行 AABB 视口相交裁剪）
                for r in 0..rows {
                    for c in 0..cols {
                        let idx = r * cols + c;
                        if let Some(tile_img) = frame.tiles.get(idx) {
                            let (tile_x, tile_y, tile_w, tile_h) =
                                transom_rdp::tiles::compute_tile_bounds(
                                    desktop_width,
                                    desktop_height,
                                    c as u32,
                                    r as u32,
                                );

                            let dst_origin = bounds.origin
                                + Point::new(
                                    px(tile_x as f32 * scale_x),
                                    px(tile_y as f32 * scale_y),
                                );
                            let dst_size = Size {
                                width: px(tile_w as f32 * scale_x),
                                height: px(tile_h as f32 * scale_y),
                            };

                            // AABB 视口相交判定：若瓦片完全落在视口外部则剔除
                            if dst_origin.x + dst_size.width <= bounds.origin.x
                                || dst_origin.x >= view_r
                                || dst_origin.y + dst_size.height <= bounds.origin.y
                                || dst_origin.y >= view_b
                            {
                                continue;
                            }

                            let tile_bounds = Bounds {
                                origin: dst_origin,
                                size: dst_size,
                            };

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

                // 2. 客户端自绘远端光标位图（零延迟且支持动态形状）
                if let Some(CursorUpdate::Custom {
                    width,
                    height,
                    hot_x,
                    hot_y,
                    image,
                }) = &cursor
                    && let Some((mx, my)) = last_mouse_pos
                {
                    let cur_x = bounds.origin.x + px(mx as f32 * scale_x - *hot_x as f32);
                    let cur_y = bounds.origin.y + px(my as f32 * scale_y - *hot_y as f32);
                    let cur_bounds = Bounds {
                        origin: Point::new(cur_x, cur_y),
                        size: Size {
                            width: px(*width as f32),
                            height: px(*height as f32),
                        },
                    };
                    let _ = window.paint_image(
                        bounds,
                        cur_bounds,
                        Corners::default(),
                        image.clone(),
                        0,
                        false,
                    );
                }
            },
        )
        .size_full()
    }
}
