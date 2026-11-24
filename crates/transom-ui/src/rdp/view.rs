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
use gpui_kit::component::Sizable;
use gpui_kit::component::button::Button;
use gpui_kit::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Instant;
use tokio::sync::mpsc;
use transom_rdp::{
    ClipboardEvent, KeyChord, RdpError, RdpInput, RdpOutput, RdpSession, SessionConfig,
};
use transom_ui_icons::{AppIcon, AppIconElement, UiIcon};

/// 会话连接显示状态。
#[derive(Debug, Clone)]
pub enum RdpViewState {
    Connecting,
    Connected { width: u16, height: u16 },
    Disconnected,
    Failed(String),
}

/// RDP 视图向上层工作区外壳发射的单向状态事件（遵循“数据向下、意图向上”契约）。
#[derive(Debug, Clone)]
pub enum RdpViewEvent {
    /// 远端桌面已连接就绪。
    Connected,
    /// 会话已正常断开。
    Disconnected,
    /// 会话连接或运行失败。
    Failed(String),
    /// 用户点击重连按钮，请求工作区外壳异步重新拉取凭据并重启会话。
    Reconnect,
}

impl EventEmitter<RdpViewEvent> for RdpView {}

/// 收敛在视口内的交互输入子状态，避免将输入细节平铺到视图顶层。
pub(crate) struct RdpInputState {
    pub(crate) last_mouse_pos: Rc<Cell<Option<(u16, u16)>>>,
    pub(crate) last_mouse_sent_at: Instant,
    pub(crate) scroll_remainder_x: f32,
    pub(crate) scroll_remainder_y: f32,
    pub(crate) pan_offset: (f32, f32),
}

impl Default for RdpInputState {
    fn default() -> Self {
        Self {
            last_mouse_pos: Rc::new(Cell::new(None)),
            last_mouse_sent_at: Instant::now(),
            scroll_remainder_x: 0.0,
            scroll_remainder_y: 0.0,
            pan_offset: (0.0, 0.0),
        }
    }
}

/// RDP 远程桌面视口视图。
pub struct RdpView {
    pub(crate) state: RdpViewState,
    pub(crate) frame_cache: Rc<RefCell<TiledFrameCache>>,
    pub(crate) input: RdpInputState,
    session: Option<RdpSession>,
    is_muted: bool,
    pub(crate) is_scaled: bool,
    pub(crate) dynamic_resolution: bool,
    pub(crate) cursor: RenderedCursor,
    pub(crate) viewport_bounds: Rc<Cell<Bounds<Pixels>>>,
    pub(crate) last_sent_resize: Rc<Cell<Option<(u16, u16)>>>,
    last_synced_clipboard: Option<String>,
    focus_handle: FocusHandle,
    _focus_subscriptions: Vec<Subscription>,
    _consumer_tasks: Vec<Task<()>>,
}

impl RdpView {
    /// 构造处于连接准备态的 RDP 视图（用于后台异步读取钥匙串后再调用 [`start_session`]）。
    pub fn new_pending(
        initial_muted: bool,
        dynamic_resolution: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            state: RdpViewState::Connecting,
            frame_cache: Rc::new(RefCell::new(TiledFrameCache::default())),
            input: RdpInputState::default(),
            session: None,
            is_muted: initial_muted,
            is_scaled: true,
            dynamic_resolution,
            cursor: RenderedCursor::Default,
            viewport_bounds: new_viewport_bounds_cell(),
            last_sent_resize: Rc::new(Cell::new(None)),
            last_synced_clipboard: None,
            focus_handle: cx.focus_handle(),
            _focus_subscriptions: Vec::new(),
            _consumer_tasks: Vec::new(),
        }
    }

    /// 构造新的 RDP 桌面视图并立即启动后台会话连接。
    #[allow(dead_code)]
    pub fn new(config: SessionConfig, cx: &mut Context<Self>) -> Self {
        let mut view = Self::new_pending(!config.audio, config.dynamic_resolution, cx);
        view.start_session(config, cx);
        view
    }

    /// 使用就绪的会话配置启动或重新启动后台 RDP 连接（原地复用标签页）。
    pub fn start_session(&mut self, config: SessionConfig, cx: &mut Context<Self>) {
        if let Some(old_session) = self.session.take() {
            old_session.disconnect();
        }
        self._consumer_tasks.clear();
        self.retire_rendered_resources();
        self.input.pan_offset = (0.0, 0.0);
        self.last_sent_resize.set(None);
        self.is_muted = !config.audio;
        self.dynamic_resolution = config.dynamic_resolution;
        self.state = RdpViewState::Connecting;

        let (session, receivers) = RdpSession::connect(config);
        let event_task = Self::spawn_event_consumer(receivers.events, cx);
        let frame_task = Self::spawn_frame_consumer(receivers.frames, cx);
        self.session = Some(session);
        self._consumer_tasks = vec![event_task, frame_task];
        cx.notify();
    }

    /// 派生低频控制事件消费协程（随视图析构自动取消）。
    fn spawn_event_consumer(
        mut events_rx: mpsc::Receiver<RdpOutput>,
        cx: &mut Context<Self>,
    ) -> Task<()> {
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
    }

    /// 派生高频桌面增量帧消费协程（随视图析构自动取消），原地更新纹理缓存并请求重绘。
    fn spawn_frame_consumer(
        mut frames_rx: mpsc::Receiver<transom_rdp::FrameUpdate>,
        cx: &mut Context<Self>,
    ) -> Task<()> {
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
    }

    /// 处理单条会话控制事件并向工作区同步生命周期状态。
    fn apply_session_event(&mut self, output: RdpOutput, cx: &mut Context<Self>) {
        match output {
            RdpOutput::Connected { width, height } => {
                self.state = RdpViewState::Connected { width, height };
                cx.emit(RdpViewEvent::Connected);
            }
            RdpOutput::Resized { width, height } => {
                self.last_sent_resize.set(Some((width, height)));
                self.state = RdpViewState::Connected { width, height };
            }
            RdpOutput::Cursor(cursor) => {
                if let RenderedCursor::Custom { image, .. } = &self.cursor {
                    self.frame_cache.borrow_mut().retire_image(image.clone());
                }
                self.cursor = RenderedCursor::from(cursor);
            }
            RdpOutput::Clipboard(ClipboardEvent::RemoteText(text)) => {
                self.last_synced_clipboard = Some(text.clone());
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            }
            RdpOutput::Disconnected { reason } => {
                log::info!("RDP 会话已断开: {reason}");
                self.session = None;
                self.retire_rendered_resources();
                self.state = RdpViewState::Disconnected;
                cx.emit(RdpViewEvent::Disconnected);
            }
            RdpOutput::Terminated(err) => {
                let detail = err.to_string();
                self.session = None;
                self.retire_rendered_resources();
                self.state = RdpViewState::Failed(detail.clone());
                cx.emit(RdpViewEvent::Failed(detail));
            }
        }
        cx.notify();
    }

    /// 获取用于 prepaint 阶段无重入发送动态分辨率请求的轻量通道克隆。
    pub(crate) fn session_sender(&self) -> Option<mpsc::Sender<RdpInput>> {
        self.session.as_ref().map(RdpSession::input_sender)
    }

    /// 聚焦当前 RDP 桌面视图以接收后续键盘输入。
    pub(crate) fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle, cx);
    }

    /// 当前是否处于客户端自绘自定义光标模式。
    pub(crate) fn is_custom_cursor_active(&self) -> bool {
        matches!(self.cursor, RenderedCursor::Custom { .. })
    }

    /// 投递最新鼠标坐标（覆盖式无阻塞槽位，绝不挤占按键通道）。
    pub fn send_mouse_move(&self, x: u16, y: u16) {
        if let Some(session) = &self.session {
            session.send_mouse_move(x, y);
        }
    }

    /// 发送输入指令到 RDP 会话循环；通道背压满时记录警告日志便于排查。
    pub fn send_input(&self, input: RdpInput) {
        if let Some(session) = &self.session
            && let Err(RdpError::ChannelFull { channel }) = session.try_send_input(input)
        {
            log::warn!("RDP 关键输入通道已满（{channel}），本次输入已丢弃");
        }
    }

    /// 原子发送预设组合键（如 Ctrl+Alt+Del、Win）。
    fn send_key_chord(&self, chord: KeyChord) {
        self.send_input(RdpInput::SendKeyChord(chord));
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

    /// 清理本地缓存瓦片与自定义光标纹理并移入待回收队列。
    fn retire_rendered_resources(&mut self) {
        let mut cache = self.frame_cache.borrow_mut();
        cache.clear();
        if let RenderedCursor::Custom { image, .. } =
            std::mem::replace(&mut self.cursor, RenderedCursor::Default)
        {
            cache.retire_image(image);
        }
    }

    /// 发送断开请求并释放按键与会话句柄。
    pub fn disconnect(&mut self) {
        self.send_input(RdpInput::ReleaseAllKeys);
        if let Some(session) = self.session.take() {
            session.disconnect();
        }
        self.retire_rendered_resources();
        self.state = RdpViewState::Disconnected;
    }

    /// 渲染状态提示视图（连接中、断开或失败，断开/失败提供一键重连入口）。
    fn render_status_view(&self, p: &Palette, cx: &Context<Self>) -> AnyElement {
        let reconnect_button = || {
            Button::new(ElementId::Name("rdp-reconnect-btn".into()))
                .small()
                .child(AppIconElement::new(AppIcon::Ui(UiIcon::REFRESH)).small())
                .label(rust_i18n::t!("tab.reconnect").to_string())
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.state = RdpViewState::Connecting;
                    cx.emit(RdpViewEvent::Reconnect);
                    cx.notify();
                }))
        };

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
                .gap_3()
                .text_color(p.danger)
                .child(rust_i18n::t!("rdp.failed", msg = msg).to_string())
                .child(reconnect_button())
                .into_any_element(),
            RdpViewState::Disconnected => v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_3()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("rdp.disconnected").to_string())
                .child(reconnect_button())
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
            on_send_cad: make_action(|this, _| this.send_key_chord(KeyChord::CtrlAltDelete)),
            on_send_win: make_action(|this, _| this.send_key_chord(KeyChord::Win)),
            on_toggle_mute: make_action(|this, cx| {
                this.is_muted = !this.is_muted;
                this.send_input(RdpInput::MuteAudio(this.is_muted));
                cx.notify();
            }),
            on_toggle_scale: make_action(|this, cx| {
                this.is_scaled = !this.is_scaled;
                if this.is_scaled {
                    this.input.pan_offset = (0.0, 0.0);
                }
                cx.notify();
            }),
            on_disconnect: make_action(|this, cx| {
                this.disconnect();
                cx.emit(RdpViewEvent::Disconnected);
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
    /// 绑定一次性焦点/失焦事件订阅，避免在每帧 `render` 中轮询焦点边沿与发起网络同步。
    fn ensure_focus_subscriptions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self._focus_subscriptions.is_empty() {
            return;
        }
        let on_focus = cx.on_focus(&self.focus_handle, window, |this, _, cx| {
            this.sync_clipboard_to_remote(cx);
        });
        let on_blur = cx.on_blur(&self.focus_handle, window, |this, _, _| {
            this.send_input(RdpInput::ReleaseAllKeys);
        });
        self._focus_subscriptions = vec![on_focus, on_blur];
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
        self.ensure_focus_subscriptions(window, cx);

        // 非连接状态下无 Canvas paint 阶段，仅当确实存在待回收纹理时执行一次清理
        if !matches!(self.state, RdpViewState::Connected { .. }) {
            self.frame_cache.borrow_mut().drain_stale_images(window);
        }

        let root = div()
            .track_focus(&self.focus_handle)
            .size_full()
            .relative()
            .bg(p.background)
            .overflow_hidden()
            .on_modifiers_changed(cx.listener(|this, e, _, _| this.handle_modifiers_changed(e)))
            .on_key_down(cx.listener(|this, e, _, cx| this.handle_key_down(e, cx)))
            .on_key_up(cx.listener(|this, e, _, _| this.handle_key_up(e)));

        match &self.state {
            RdpViewState::Connected { .. } => root
                .child(self.render_interactive_canvas(cx))
                .child(self.render_floating_toolbar(&p, cx)),
            _ => root.child(self.render_status_view(&p, cx)),
        }
    }
}

impl RdpView {
    /// 渲染绑定鼠标与滚轮事件的桌面画布容器，与悬浮工具栏物理隔离，防止工具栏点击穿透到远端。
    fn render_interactive_canvas(&self, cx: &mut Context<Self>) -> Div {
        let mut container = div()
            .size_full()
            .cursor(CursorStyle::Arrow)
            .on_mouse_move(cx.listener(|this, e, _, cx| this.handle_mouse_move(e, cx)))
            .on_mouse_exit(cx.listener(|this, _, _, cx| this.handle_mouse_exit(cx)))
            .on_scroll(cx.listener(|this, e, _, cx| this.handle_scroll(e, cx)))
            .child(self.render_desktop_canvas());

        let buttons = [
            MouseButton::Left,
            MouseButton::Right,
            MouseButton::Middle,
            MouseButton::Navigate(NavigationDirection::Back),
            MouseButton::Navigate(NavigationDirection::Forward),
        ];
        for btn in buttons {
            container = container
                .on_mouse_down(
                    btn,
                    cx.listener(move |this, _, window, cx| {
                        this.handle_mouse_down(btn, window, cx);
                    }),
                )
                .on_mouse_up(
                    btn,
                    cx.listener(move |this, _, _, _| {
                        this.handle_mouse_up(btn);
                    }),
                );
        }

        container
    }
}
