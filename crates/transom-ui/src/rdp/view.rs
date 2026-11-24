//! RDP 桌面视口核心组件与生命周期管理。
//!
//! 设计原理（Why）：作为 GPUI 自绘树中的一等公民视图，`RdpView` 维护双轨异步监听
//! （低频控制事件 + 高频增量瓦片帧）、常驻纹理缓存 [`TiledFrameCache`] 与焦点失焦
//! 按键清理；渲染层统一使用 [`Palette`] 主题语义色与设计令牌，彻底消除硬编码颜色。

use super::canvas::new_viewport_bounds_cell;
use super::frame_cache::{RenderedCursor, TiledFrameCache};
use super::toolbar::{RdpToolbarParams, render_rdp_toolbar};
use crate::palette::Palette;
use gpui_kit::base::v_flex;
use gpui_kit::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Instant;
use tokio::sync::mpsc;
use transom_rdp::{ClipboardEvent, RdpInput, RdpOutput, RdpSession, SessionConfig};

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
    pub(crate) state: RdpViewState,
    pub(crate) frame_cache: Rc<RefCell<TiledFrameCache>>,
    session: Option<RdpSession>,
    is_muted: bool,
    pub(crate) is_scaled: bool,
    pub(crate) dynamic_resolution: bool,
    pub(crate) last_mouse_pos: Option<(u16, u16)>,
    pub(crate) last_mouse_sent_at: Instant,
    pub(crate) scroll_remainder_x: f32,
    pub(crate) scroll_remainder_y: f32,
    pub(crate) cursor: RenderedCursor,
    pub(crate) viewport_bounds: Rc<Cell<Bounds<Pixels>>>,
    last_synced_clipboard: Option<String>,
    was_focused: bool,
    focus_handle: FocusHandle,
}

impl RdpView {
    /// 构造新的 RDP 桌面视图并启动后台会话连接。
    pub fn new(config: SessionConfig, cx: &mut Context<Self>) -> Self {
        let initial_muted = !config.audio;
        let dynamic_resolution = config.dynamic_resolution;
        let (session, receivers) = RdpSession::connect(config);

        Self::spawn_event_consumer(receivers.events, cx);
        Self::spawn_frame_consumer(receivers.frames, cx);

        Self {
            state: RdpViewState::Connecting,
            frame_cache: Rc::new(RefCell::new(TiledFrameCache::default())),
            session: Some(session),
            is_muted: initial_muted,
            is_scaled: true,
            dynamic_resolution,
            last_mouse_pos: None,
            last_mouse_sent_at: Instant::now(),
            scroll_remainder_x: 0.0,
            scroll_remainder_y: 0.0,
            cursor: RenderedCursor::Default,
            viewport_bounds: new_viewport_bounds_cell(),
            last_synced_clipboard: None,
            was_focused: false,
            focus_handle: cx.focus_handle(),
        }
    }

    /// 派生低频控制事件消费协程（连接、重设分辨率、光标、剪贴板、断开与错误）。
    fn spawn_event_consumer(mut events_rx: mpsc::Receiver<RdpOutput>, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            while let Some(output) = events_rx.recv().await {
                let updated = this.update(cx, |view, cx| {
                    view.apply_session_event(output, cx);
                });
                if updated.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    /// 派生高频桌面增量帧消费协程，直接原地更新纹理缓存并请求重绘。
    fn spawn_frame_consumer(
        mut frames_rx: mpsc::Receiver<transom_rdp::FrameUpdate>,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            while let Some(update) = frames_rx.recv().await {
                let updated = this.update(cx, |view, cx| {
                    view.frame_cache.borrow_mut().apply_update(update);
                    cx.notify();
                });
                if updated.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    /// 处理单条会话控制事件。
    fn apply_session_event(&mut self, output: RdpOutput, cx: &mut Context<Self>) {
        match output {
            RdpOutput::Connected { width, height } => {
                self.state = RdpViewState::Connected { width, height };
            }
            RdpOutput::Resized { width, height } => {
                self.frame_cache.borrow_mut().clear();
                self.state = RdpViewState::Connected { width, height };
            }
            RdpOutput::Cursor(cursor) => {
                self.cursor = RenderedCursor::from(cursor);
            }
            RdpOutput::Clipboard(ClipboardEvent::RemoteText(text)) => {
                self.last_synced_clipboard = Some(text.clone());
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            }
            RdpOutput::Disconnected { reason } => {
                log::info!("RDP 会话已断开: {reason}");
                self.session = None;
                self.state = RdpViewState::Disconnected;
            }
            RdpOutput::Terminated(err) => {
                self.session = None;
                self.state = RdpViewState::Failed(err.to_string());
            }
        }
        cx.notify();
    }

    /// 获取用于 prepaint 阶段无重入发送动态分辨率请求的轻量通道克隆。
    pub(crate) fn session_sender(&self) -> Option<mpsc::Sender<RdpInput>> {
        self.session.as_ref().map(RdpSession::input_sender)
    }

    /// 当前是否处于客户端自绘自定义光标模式。
    pub(crate) fn is_custom_cursor_active(&self) -> bool {
        matches!(self.cursor, RenderedCursor::Custom { .. })
    }

    /// 发送输入指令到 RDP 会话循环。
    pub fn send_input(&self, input: RdpInput) {
        if let Some(session) = &self.session {
            let _ = session.try_send_input(input);
        }
    }

    /// 发送组合键按下与抬起序列（如 Ctrl+Alt+Del、Win）。
    fn send_key_chord(&self, keys: &[&str]) {
        for &key in keys {
            self.send_input(RdpInput::KeyDown {
                key: key.to_string(),
            });
        }
        for &key in keys.iter().rev() {
            self.send_input(RdpInput::KeyUp {
                key: key.to_string(),
            });
        }
    }

    /// 检查并同步本地剪贴板新内容到远端桌面。
    pub fn sync_clipboard_to_remote(&mut self, cx: &mut Context<Self>) {
        if let Some(item) = cx.read_from_clipboard()
            && let Some(text) = item.text()
            && self.last_synced_clipboard.as_deref() != Some(&text)
        {
            self.last_synced_clipboard = Some(text.clone());
            self.send_input(RdpInput::ClipboardText(text));
        }
    }

    /// 发送断开请求并释放按键与会话句柄。
    pub fn disconnect(&mut self) {
        self.send_input(RdpInput::ReleaseAllKeys);
        if let Some(session) = self.session.take() {
            session.disconnect();
        }
        self.state = RdpViewState::Disconnected;
    }

    /// 渲染状态提示视图（连接中、断开或失败）。
    fn render_status_view(&self, p: &Palette) -> AnyElement {
        match &self.state {
            RdpViewState::Connecting => v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("rdp.connecting").to_string())
                .into_any_element(),
            RdpViewState::Failed(msg) => v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_2()
                .text_color(p.danger)
                .child(rust_i18n::t!("rdp.failed", msg = msg).to_string())
                .into_any_element(),
            RdpViewState::Disconnected => v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("rdp.disconnected").to_string())
                .into_any_element(),
            RdpViewState::Connected { .. } => div().into_any_element(),
        }
    }

    /// 渲染悬浮工具栏。
    fn render_floating_toolbar(&self, p: &Palette, cx: &Context<Self>) -> AnyElement {
        let view = cx.entity().clone();
        let make_action = |f: fn(&mut RdpView, &mut Context<RdpView>)| {
            let entity = view.clone();
            Box::new(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                entity.update(cx, f);
            }) as super::toolbar::RdpToolbarCallback
        };

        let toolbar = render_rdp_toolbar(RdpToolbarParams {
            palette: p,
            is_muted: self.is_muted,
            is_scaled: self.is_scaled,
            on_send_cad: make_action(|this, _| this.send_key_chord(&["ctrl", "alt", "delete"])),
            on_send_win: make_action(|this, _| this.send_key_chord(&["super"])),
            on_toggle_mute: make_action(|this, cx| {
                this.is_muted = !this.is_muted;
                this.send_input(RdpInput::MuteAudio(this.is_muted));
                cx.notify();
            }),
            on_toggle_scale: make_action(|this, cx| {
                this.is_scaled = !this.is_scaled;
                cx.notify();
            }),
            on_disconnect: make_action(|this, cx| {
                this.disconnect();
                cx.notify();
            }),
        });

        div()
            .absolute()
            .top_2()
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::from(cx);

        // 监测焦点从聚焦转为失焦的边沿，及时向远端释放全部按下键位，避免修饰键粘滞
        let is_focused = self.focus_handle.is_focused(window);
        if self.was_focused && !is_focused {
            self.send_input(RdpInput::ReleaseAllKeys);
        }
        self.was_focused = is_focused;

        let root = div()
            .track_focus(&self.focus_handle)
            .size_full()
            .relative()
            .bg(p.background)
            .cursor(CursorStyle::Arrow)
            .overflow_hidden()
            .on_mouse_move(cx.listener(|this, e, _, cx| this.handle_mouse_move(e, cx)))
            .on_mouse_exit(cx.listener(|this, _, _, cx| this.handle_mouse_exit(cx)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.handle_mouse_down(MouseButton::Left, cx)),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.handle_mouse_up(MouseButton::Left)),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, _, _, cx| this.handle_mouse_down(MouseButton::Right, cx)),
            )
            .on_mouse_up(
                MouseButton::Right,
                cx.listener(|this, _, _, _| this.handle_mouse_up(MouseButton::Right)),
            )
            .on_mouse_down(
                MouseButton::Middle,
                cx.listener(|this, _, _, cx| this.handle_mouse_down(MouseButton::Middle, cx)),
            )
            .on_mouse_up(
                MouseButton::Middle,
                cx.listener(|this, _, _, _| this.handle_mouse_up(MouseButton::Middle)),
            )
            .on_scroll(cx.listener(|this, e, _, _| this.handle_scroll(e)))
            .on_key_down(cx.listener(|this, e, _, cx| this.handle_key_down(e, cx)))
            .on_key_up(cx.listener(|this, e, _, _| this.handle_key_up(e)));

        match &self.state {
            RdpViewState::Connected { .. } => root
                .child(self.render_desktop_canvas())
                .child(self.render_floating_toolbar(&p, cx)),
            _ => root.child(self.render_status_view(&p)),
        }
    }
}
