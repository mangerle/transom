//! RDP 桌面视口核心组件与事件处理。
//!
//! 设计原理（Why）：作为 GPUI 自绘树中的一等公民视图，`RdpView` 采用原生 GPU
//! `canvas` 管道对后台会话线程交付的增量瓦片帧（`TiledDesktopFrame`）进行直接渲染，
//! 消除数百个 DOM 节点开销；在 prepaint 阶段捕获精确物理视口矩形，实现鼠标坐标
//! 的无偏差绝对反算与动态缩放映射；打通系统剪贴板（GPUI Clipboard）双向流转。

use gpui_kit::base::v_flex;
use gpui_kit::*;
use std::sync::Arc;
use transom_core::SessionId;
use transom_rdp::{
    ClipboardEvent, CursorUpdate, RdpInput, RdpOutput, RdpSession, SessionConfig, TiledDesktopFrame,
};

use super::toolbar::{RdpToolbarParams, render_rdp_toolbar};

/// 会话连接显示状态。
#[derive(Debug, Clone)]
pub enum RdpViewState {
    Connecting,
    Connected { width: u16, height: u16 },
    Disconnected,
    Failed(String),
}

/// RDP 远程桌面视口视图。
pub struct RdpView {
    #[allow(dead_code)]
    pub session_id: SessionId,
    #[allow(dead_code)]
    pub title: String,
    state: RdpViewState,
    frame: Option<Arc<TiledDesktopFrame>>,
    session: Option<RdpSession>,
    is_muted: bool,
    is_scaled: bool,
    last_mouse_pos: Option<(u16, u16)>,
    #[allow(dead_code)]
    cursor: Option<CursorUpdate>,
    viewport_bounds: Bounds<Pixels>,
    focus_handle: FocusHandle,
}

impl RdpView {
    /// 构造新的 RDP 桌面视图。
    pub fn new(
        session_id: SessionId,
        title: String,
        config: SessionConfig,
        cx: &mut Context<Self>,
    ) -> Self {
        let (session, mut output_rx) = RdpSession::connect(config);

        // 监听来自后台会话线程的输出通知
        cx.spawn(async move |this, cx| {
            while let Some(output) = output_rx.recv().await {
                let res = this.update(cx, |view, cx| {
                    match output {
                        RdpOutput::Connected { width, height } => {
                            view.state = RdpViewState::Connected { width, height };
                        }
                        RdpOutput::Frame(f) => {
                            view.frame = Some(f);
                        }
                        RdpOutput::Cursor(c) => {
                            view.cursor = Some(c);
                        }
                        RdpOutput::Clipboard(ClipboardEvent::RemoteText(text)) => {
                            cx.write_to_clipboard(ClipboardItem::new_string(text));
                        }
                        RdpOutput::Terminated(e) => {
                            view.state = RdpViewState::Failed(e.to_string());
                        }
                        _ => {}
                    }
                    cx.notify();
                });
                if res.is_err() {
                    break;
                }
            }
        })
        .detach();

        Self {
            session_id,
            title,
            state: RdpViewState::Connecting,
            frame: None,
            session: Some(session),
            is_muted: false,
            is_scaled: true,
            last_mouse_pos: None,
            cursor: None,
            viewport_bounds: Bounds::default(),
            focus_handle: cx.focus_handle(),
        }
    }

    /// 发送输入指令到 RDP 会话循环。
    pub fn send_input(&self, input: RdpInput) {
        if let Some(session) = &self.session {
            let _ = session.try_send_input(input);
        }
    }

    /// 发送断开请求并释放按键。
    pub fn disconnect(&mut self) {
        self.send_input(RdpInput::ReleaseAllKeys);
        if let Some(session) = &self.session {
            session.disconnect();
        }
        self.state = RdpViewState::Disconnected;
    }

    /// 渲染状态提示视图（连接中、断开或失败）。
    fn render_status_view(&self) -> AnyElement {
        match &self.state {
            RdpViewState::Connecting => v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .text_color(gpui_kit::rgb(0xaaaaaa))
                .child(rust_i18n::t!("rdp.connecting").to_string())
                .into_any_element(),
            RdpViewState::Failed(msg) => v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap(px(8.0))
                .text_color(gpui_kit::rgb(0xff5555))
                .child(rust_i18n::t!("rdp.failed", msg = msg).to_string())
                .into_any_element(),
            RdpViewState::Disconnected => v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .text_color(gpui_kit::rgb(0x888888))
                .child(rust_i18n::t!("rdp.disconnected").to_string())
                .into_any_element(),
            RdpViewState::Connected { .. } => div().into_any_element(),
        }
    }

    /// 渲染桌面瓦片集合的高性能 GPU 自绘画布。
    fn render_desktop_canvas(&self, cx: &Context<Self>) -> impl IntoElement {
        let view_entity = cx.entity().clone();
        let frame = self.frame.clone();
        let is_scaled = self.is_scaled;

        canvas(
            move |bounds, _window, cx| {
                view_entity.update(cx, |this, _| {
                    this.viewport_bounds = bounds;
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
            },
        )
        .size_full()
    }

    /// 渲染悬浮工具栏。
    fn render_floating_toolbar(&self, cx: &Context<Self>) -> AnyElement {
        let view_entity = cx.entity().clone();
        let toolbar = render_rdp_toolbar(RdpToolbarParams {
            is_muted: self.is_muted,
            is_scaled: self.is_scaled,
            on_send_cad: {
                let view = view_entity.clone();
                Box::new(move |_, _, cx| {
                    view.update(cx, |this, _| {
                        this.send_input(RdpInput::KeyDown {
                            key: "ctrl".to_string(),
                        });
                        this.send_input(RdpInput::KeyDown {
                            key: "alt".to_string(),
                        });
                        this.send_input(RdpInput::KeyDown {
                            key: "delete".to_string(),
                        });
                        this.send_input(RdpInput::KeyUp {
                            key: "delete".to_string(),
                        });
                        this.send_input(RdpInput::KeyUp {
                            key: "alt".to_string(),
                        });
                        this.send_input(RdpInput::KeyUp {
                            key: "ctrl".to_string(),
                        });
                    });
                })
            },
            on_toggle_mute: {
                let view = view_entity.clone();
                Box::new(move |_, _, cx| {
                    view.update(cx, |this, cx| {
                        this.is_muted = !this.is_muted;
                        this.send_input(RdpInput::MuteAudio(this.is_muted));
                        cx.notify();
                    });
                })
            },
            on_toggle_scale: {
                let view = view_entity.clone();
                Box::new(move |_, _, cx| {
                    view.update(cx, |this, cx| {
                        this.is_scaled = !this.is_scaled;
                        cx.notify();
                    });
                })
            },
            on_disconnect: {
                let view = view_entity.clone();
                Box::new(move |_, _, cx| {
                    view.update(cx, |this, cx| {
                        this.disconnect();
                        cx.notify();
                    });
                })
            },
        });

        div()
            .absolute()
            .top(px(8.0))
            .left_0()
            .right_0()
            .flex()
            .justify_center()
            .child(toolbar)
            .into_any_element()
    }
}

impl Focusable for RdpView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for RdpView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let root = div()
            .track_focus(&self.focus_handle)
            .size_full()
            .relative()
            .bg(gpui_kit::rgb(0x101010))
            .overflow_hidden()
            .on_mouse_move(cx.listener(move |this, event: &MouseMoveEvent, _, _| {
                if let RdpViewState::Connected { width, height } = this.state
                    && let Some((x, y)) = crate::rdp::input::map_pointer_position(
                        event.position,
                        this.viewport_bounds,
                        width,
                        height,
                        this.is_scaled,
                    )
                    && this.last_mouse_pos != Some((x, y))
                {
                    this.last_mouse_pos = Some((x, y));
                    this.send_input(RdpInput::MouseMove { x, y });
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseDownEvent, _, _| {
                    if let Some(input) =
                        crate::rdp::input::map_mouse_button(MouseButton::Left, true)
                    {
                        this.send_input(input);
                    }
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseUpEvent, _, _| {
                    if let Some(input) =
                        crate::rdp::input::map_mouse_button(MouseButton::Left, false)
                    {
                        this.send_input(input);
                    }
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, _: &MouseDownEvent, _, _| {
                    if let Some(input) =
                        crate::rdp::input::map_mouse_button(MouseButton::Right, true)
                    {
                        this.send_input(input);
                    }
                }),
            )
            .on_mouse_up(
                MouseButton::Right,
                cx.listener(move |this, _: &MouseUpEvent, _, _| {
                    if let Some(input) =
                        crate::rdp::input::map_mouse_button(MouseButton::Right, false)
                    {
                        this.send_input(input);
                    }
                }),
            )
            .on_mouse_down(
                MouseButton::Middle,
                cx.listener(move |this, _: &MouseDownEvent, _, _| {
                    if let Some(input) =
                        crate::rdp::input::map_mouse_button(MouseButton::Middle, true)
                    {
                        this.send_input(input);
                    }
                }),
            )
            .on_mouse_up(
                MouseButton::Middle,
                cx.listener(move |this, _: &MouseUpEvent, _, _| {
                    if let Some(input) =
                        crate::rdp::input::map_mouse_button(MouseButton::Middle, false)
                    {
                        this.send_input(input);
                    }
                }),
            )
            .on_scroll(cx.listener(move |this, event: &ScrollEvent, _, _| {
                let (delta_x, delta_y) = match event.delta {
                    ScrollDelta::Lines(p) => ((p.x * 120.0) as i16, (p.y * 120.0) as i16),
                    ScrollDelta::Pixels(p) => {
                        let dy = (f32::from(p.y) / 10.0 * 120.0) as i16;
                        let dx = (f32::from(p.x) / 10.0 * 120.0) as i16;
                        (dx, dy)
                    }
                };
                if delta_y != 0 {
                    this.send_input(RdpInput::MouseWheel {
                        vertical: true,
                        delta: delta_y,
                    });
                }
                if delta_x != 0 {
                    this.send_input(RdpInput::MouseWheel {
                        vertical: false,
                        delta: delta_x,
                    });
                }
            }))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, _| {
                this.send_input(RdpInput::KeyDown {
                    key: event.keystroke.key.clone(),
                });
            }))
            .on_key_up(cx.listener(move |this, event: &KeyUpEvent, _, _| {
                this.send_input(RdpInput::KeyUp {
                    key: event.keystroke.key.clone(),
                });
            }));

        match &self.state {
            RdpViewState::Connected { .. } => root
                .child(self.render_desktop_canvas(cx))
                .child(self.render_floating_toolbar(cx)),
            _ => root.child(self.render_status_view()),
        }
    }
}
