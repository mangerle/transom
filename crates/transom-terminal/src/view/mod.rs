mod block;
mod clipboard;
mod copy_mode;
mod hit_test;
mod input;
mod integration;
mod links;
mod menu;
mod paint;
mod recording;
mod render;
mod silent_echo;
mod tasks;
#[cfg(test)]
mod tests;
mod types;

use crate::engine::TerminalEngine;
use crate::event::{HistoryProvider, TerminalEvent};
use crate::input::focus_bytes;
use crate::osc::{CommandTracker, TitleGate};
use crate::record::RecorderHandle;
use crate::renderer::TerminalRenderer;
use crate::shell_integration::ShellIntegration;
use alacritty_terminal::event::Event;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point as AlacPoint};
use alacritty_terminal::term::TermMode;
use alacritty_terminal::vte::ansi::CursorShape;
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::input::InputState;
use gpui_kit::*;
pub(crate) use input::build_custom_keys;
use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use transom_core::{
    BlockViewSettings, SessionCleanup, TerminalControlHandle, TerminalInteraction, TerminalIo,
    TerminalProfile, TerminalSize,
};
/// `CONTEXT` 对外以 `TERMINAL_CONTEXT` 暴露：应用侧的可改键表需要按同一上下文
/// 注册终端专属动作，裸名 `CONTEXT` 在 crate 之外无法辨认它指的是哪个域。
pub use types::{CONTEXT, KeyFilter};
pub(crate) use types::{
    LayoutMetrics, PendingResize, PreparedTerminal, READ_BUFFER_BYTES, READ_CHANNEL_CAPACITY,
    RESIZE_DEBOUNCE, ResizeDecision, SEARCH_TICK_PIXELS, TerminalRenderState, decide_resize,
};

actions!(
    transom_terminal,
    [SendTab, SendBacktab, TerminalCopy, TerminalPaste]
);

pub struct TerminalView {
    /// 终端渲染与引擎核心状态聚合容器。
    pub(crate) state: Arc<TerminalRenderState>,
    profile: TerminalProfile,
    interaction: TerminalInteraction,
    focus_handle: FocusHandle,
    input_tx: flume::Sender<Vec<u8>>,
    /// 待解析的终端输出通道接收端。视图与读取任务各持一份：粘贴前视图用它把
    /// 已到达但还没解析的输出先并入引擎，保证模式判定尽量贴近写入时刻。
    output_rx: flume::Receiver<Vec<u8>>,
    control: TerminalControlHandle,
    event_rx: flume::Receiver<Event>,
    key_filter: Option<KeyFilter>,
    pub(crate) ime_text: String,
    pub(crate) ime_marked_range: Option<Range<usize>>,
    pub(crate) dragging_selection: bool,
    pub(crate) mouse_pressed: bool,
    pub(crate) exited: bool,
    /// 当前闪烁相位；关闭闪烁或失焦时始终保持为 `true`。
    cursor_blink_on: bool,
    /// 像素滚动的未凑整余数（单位：行）。触摸板逐事件位移常小于一行，
    /// 不累积的话 round 会把小于半行的事件全部舍成 0，平滑滚动失效。
    scroll_pixel_remainder: f32,
    /// 悬浮检索栏是否展开。
    pub(crate) search_open: bool,
    /// 检索输入框状态；首次展开时创建，收起后保留上次的查询词。
    pub(crate) search_input: Option<Entity<InputState>>,
    /// 命令生命周期归并：把 OSC 133 的零散标记算成"一条命令跑了多久"。
    command_tracker: CommandTracker,
    /// 运行期标题门控：区分提示符标题与命令/TUI 标题。
    title_gate: TitleGate,
    /// 本会话是否已被用户授权写入本地剪贴板（OSC 52）。
    ///
    /// 只活在视图里，不落盘：断开重连会创建新视图，授权随之失效，避免一次
    /// 批准变成永久放行。默认拒绝，远端请求必须先经过宿主的确认弹窗。
    remote_clipboard_allowed: bool,
    /// 键盘复制模式的选区锚点：按 `v`/`V` 时的光标位置。
    ///
    /// 只在视图侧维护：内核的 `Selection` 只记录起点与终点，不知道"是否处于
    /// 选择中"，而 `v` 的第二次按下要撤销选区，需要这个显式状态。
    copy_anchor: Option<AlacPoint>,
    /// Shell 集成流程控制器；进入终态（已生效或已放弃）后轮询任务自行退出。
    shell_integration: ShellIntegration,
    /// 用户是否已亲自按键过。一旦按键，本轮不再注入：写进去的字节会拼到用户
    /// 半截的命令行上，把那条命令毁掉。状态机宁可不装也不毁命令。
    user_typed: bool,
    /// 命令历史候选查询；未挂接时建议功能整体不生效。
    pub(crate) history_provider: Option<HistoryProvider>,
    /// 本地跟踪的当前输入行，只用于取建议前缀。
    pub(crate) input_line: String,
    /// 当前候选列表与选中位置（Tab 逐个采纳，不抢上下键）。
    pub(crate) suggestions: Vec<String>,
    pub(crate) suggestion_index: usize,
    /// 是否把查询词当正则表达式解析（与引擎里的快照同步）。
    pub(crate) search_regex_mode: bool,
    /// 检索输入框的事件订阅；随实体存活，收起搜索栏也不释放（下次复用同一输入框）。
    pub(crate) _search_subscription: Option<Subscription>,
    /// 语义块视图配置；`None` 或 `enabled == false` 时走经典连续流。
    block_view: Option<BlockViewSettings>,
    /// 当前悬浮的命令块与所在可见行；`None` 表示不在块上。
    hover_block: Option<(u64, usize)>,
    /// 指针是否停在可打开的 OSC 8 链接上；仅用于避免重复提交光标样式。
    hover_link: bool,
    /// 已解析的自定义按键映射；键盘热路径只做遍历匹配。
    custom_keys: Vec<(KeybindingKeystroke, Vec<u8>)>,
    /// 尺寸去抖计时任务：到点唤醒一帧，让待下发的网格尺寸真正落地。
    ///
    /// 新的目标尺寸会顶掉上一个计时（丢弃句柄即取消），因此连续拖拽只保留最后一个。
    resize_tick: Option<Task<()>>,
    /// 会话级清理守卫：随视图存活，视图销毁时才释放会话的临时资源
    /// （如本地 Shell 预装用的临时 rcfile 目录）。
    _cleanup: SessionCleanup,
    _reader_task: Task<()>,
    _blink_task: Task<()>,
    /// Shell 集成轮询任务句柄；终态或视图回收后自行结束。
    _integration_task: Task<()>,
    _focus_subscriptions: Vec<Subscription>,
    recorder: RecorderHandle,
}

/// 终端视图初始化参数结构体。
pub struct TerminalViewParams {
    pub io: TerminalIo,
    pub profile: TerminalProfile,
    pub interaction: TerminalInteraction,
    pub key_filter: Option<KeyFilter>,
    pub audit_path: Option<std::path::PathBuf>,
    /// 是否启用 Shell 集成（提示符导航与退出码识别）。
    ///
    /// 随会话创建参数带入，而不是让视图回头去读设置：视图诞生时必须已经知道
    /// 要不要集成，否则会先发一次探针再被设置关掉，探针本身是可见的。
    pub shell_integration_enabled: bool,
    /// 首帧尺寸下发后清屏一次：容器内的 `sh` 收到 SIGWINCH 会把提示符重新
    /// 打印到新行（真机复现：每次 resize 多一行 `#`）。控制台会话没有登录欢迎
    /// 信息，清掉这些重复提示符即可；普通终端标签保持原样，不能抹掉 banner。
    pub clear_on_first_resize: bool,
}

impl TerminalView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>, params: TerminalViewParams) -> Self {
        let TerminalViewParams {
            io,
            profile,
            interaction,
            key_filter,
            audit_path,
            shell_integration_enabled,
            clear_on_first_resize,
        } = params;
        cx.bind_keys([
            KeyBinding::new("tab", SendTab, Some(CONTEXT)),
            KeyBinding::new("shift-tab", SendBacktab, Some(CONTEXT)),
        ]);

        let focus_handle = cx.focus_handle().tab_stop(false);
        let (event_tx, event_rx) = flume::bounded(256);
        let renderer = TerminalRenderer::new(&profile, window, cx);
        let engine = TerminalEngine::new(TerminalSize::new(80, 24), &profile, event_tx.clone());
        let state = Arc::new(TerminalRenderState::new(engine, renderer));
        state
            .clear_on_first_resize
            .store(clear_on_first_resize, std::sync::atomic::Ordering::Relaxed);
        let custom_keys = build_custom_keys(&profile.key_bindings);

        let recorder = RecorderHandle::new();
        let TerminalIo {
            reader,
            writer,
            control,
            integration_preinstalled,
            cleanup,
        } = io;
        // 集成已在会话启动阶段预装时直接落终态：终端侧不再发探针，也不再注入。
        let shell_integration = match integration_preinstalled {
            Some(kind) => ShellIntegration::preinstalled(kind),
            None => ShellIntegration::new(shell_integration_enabled),
        };
        let (input_tx, output_rx, reader_task) = Self::spawn_io_tasks(
            tasks::TerminalIoTaskParams {
                reader,
                writer,
                audit_path,
                recorder: &recorder,
                event_tx: &event_tx,
                state: &state,
            },
            cx,
        );

        let on_focus = cx.on_focus(&focus_handle, window, |view, _, cx| {
            let mode = view.state.engine.lock().mode();
            if let Some(bytes) = focus_bytes(mode, true) {
                view.write_input(&bytes);
            }
            view.cursor_blink_on = true;
            cx.notify();
        });
        let on_blur = cx.on_blur(&focus_handle, window, |view, _, cx| {
            let mode = view.state.engine.lock().mode();
            if let Some(bytes) = focus_bytes(mode, false) {
                view.write_input(&bytes);
            }
            view.cursor_blink_on = true;
            cx.notify();
        });

        // 闪烁只驱动光标相位，不触碰终端内容；失焦或关闭闪烁时定时器保持静默。
        let blink_task = cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(530))
                    .await;
                let updated = this.update_in(cx, |view, window, cx| {
                    view.tick_cursor_blink(window, cx);
                });
                if updated.is_err() {
                    break;
                }
            }
        });

        // Shell 集成：低频轮询到安全时机后注入 OSC 133 钩子，终态后任务自行退出。
        let integration_task = Self::spawn_integration_task(cx);

        Self {
            state,
            profile,
            interaction,
            focus_handle,
            input_tx,
            output_rx,
            control,
            event_rx,
            key_filter,
            ime_text: String::new(),
            ime_marked_range: None,
            dragging_selection: false,
            mouse_pressed: false,
            exited: false,
            cursor_blink_on: true,
            scroll_pixel_remainder: 0.0,
            command_tracker: CommandTracker::default(),
            title_gate: TitleGate::default(),
            remote_clipboard_allowed: false,
            copy_anchor: None,
            shell_integration,
            user_typed: false,
            history_provider: None,
            input_line: String::new(),
            suggestions: Vec::new(),
            suggestion_index: 0,
            search_open: false,
            search_input: None,
            search_regex_mode: false,
            _search_subscription: None,
            block_view: None,
            hover_block: None,
            hover_link: false,
            custom_keys,
            resize_tick: None,
            _cleanup: cleanup,
            _reader_task: reader_task,
            _blink_task: blink_task,
            _integration_task: integration_task,
            _focus_subscriptions: vec![on_focus, on_blur],
            recorder,
        }
    }

    /// 从设置重载当前终端的触发器规则。
    pub fn reload_triggers(&mut self, settings: &transom_core::TriggerSettings) {
        self.state.trigger_engine.lock().reload(settings);
    }

    /// 用户在本会话内允许远端写入本地剪贴板（OSC 52）。
    ///
    /// 只提升当前视图的授权位：不触发重绘、不写剪贴板——待写入的内容由宿主
    /// 在批准时自行落盘，这里只负责让后续请求不再进入确认流程。
    pub fn allow_remote_clipboard_write(&mut self) {
        self.remote_clipboard_allowed = true;
    }

    /// 排一次尺寸去抖计时：到点唤醒一帧，让预绘制阶段把待下发尺寸真正落地。
    ///
    /// 每次登记新的目标尺寸都会顶掉上一个计时（丢弃句柄即取消），
    /// 所以连续拖拽过程中只有稳定后的最后一个尺寸会被真正下发。
    fn schedule_resize_tick(&mut self, cx: &mut Context<Self>) {
        self.resize_tick = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(RESIZE_DEBOUNCE).await;
            let _ = this.update(cx, |_, cx| cx.notify());
        }));
    }

    fn tick_cursor_blink(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let blinking = self.focus_handle.is_focused(window) && self.profile.cursor_blink && {
            let engine = self.state.engine.lock();
            let style = engine.term().cursor_style();
            style.blinking
                && style.shape != CursorShape::Hidden
                && engine.mode().contains(TermMode::SHOW_CURSOR)
        };
        if blinking {
            self.cursor_blink_on = !self.cursor_blink_on;
            cx.notify();
        } else if !self.cursor_blink_on {
            self.cursor_blink_on = true;
            cx.notify();
        }
    }

    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus_handle
    }

    /// 热应用全局外观偏好：字体族、中文字体族、字号、行高与关键词高亮规则。
    ///
    /// 渲染器是长生命周期缓存，改设置必须显式失效：这里置脏标记，下一帧
    /// prepaint（持有窗口文本系统）按新 profile 重建度量与编译规则。调色板、
    /// scrollback 与光标外观不在这里覆盖，那些仍归会话自己的 profile，
    /// 避免全局偏好悄悄吃掉会话级配置。
    /// 网格列数会随字号变化，由下一帧 prepaint 统一走既有的 resize + PTY 同步通路。
    pub fn apply_appearance(&mut self, profile: &TerminalProfile, cx: &mut Context<Self>) {
        self.profile.font_family = profile.font_family.clone();
        self.profile.cjk_font_family = profile.cjk_font_family.clone();
        self.profile.font_size = profile.font_size;
        self.profile.line_height = profile.line_height;
        self.profile.highlight = profile.highlight.clone();
        self.profile.alternate_scroll = profile.alternate_scroll;
        // 终端配色同样全局生效：界面上没有会话级配色入口，保存设置后活窗格
        // 立即换色；运行期程序经 OSC 4/10/11 的动态改色走另一条运行时表，
        // 不受此静态基线覆盖影响。
        self.profile.theme_name = profile.theme_name.clone();
        self.profile.palette = profile.palette.clone();
        if self.profile.key_bindings != profile.key_bindings {
            self.profile.key_bindings = profile.key_bindings.clone();
            self.custom_keys = build_custom_keys(&self.profile.key_bindings);
        }
        // 背景透明度参与渲染：折算成背景色的 alpha，透出下层应用背景。
        self.profile.background_transparency = profile.background_transparency;
        self.state.renderer_dirty.store(true, Ordering::Release);
        cx.notify();
    }

    /// 查找按键组合对应的自定义字节序列。
    fn custom_key_sequence(&self, keystroke: &Keystroke) -> Option<Vec<u8>> {
        self.custom_keys
            .iter()
            .find(|(binding, _)| keystroke.should_match(binding))
            .map(|(_, bytes)| bytes.clone())
    }

    pub(crate) fn write_input(&mut self, bytes: &[u8]) -> bool {
        if bytes.is_empty() {
            return true;
        }
        // 输入瞬间强制点亮光标，避免用户打字时恰好处于熄灭相位。
        self.cursor_blink_on = true;
        self.recorder.record_input(bytes);
        match self.input_tx.try_send(bytes.to_vec()) {
            Ok(()) => true,
            // 256 深度对人速按键与整批粘贴绰绰有余，打满只可能是写入线程
            // 卡死：此时丢掉本批输入并继续存活判定，比在 UI 线程阻塞等待好。
            Err(flume::TrySendError::Full(_)) => {
                log::warn!("终端输入队列已满，本批输入被丢弃");
                true
            }
            Err(flume::TrySendError::Disconnected(_)) => false,
        }
    }
}

impl EventEmitter<TerminalEvent> for TerminalView {}

impl Drop for TerminalView {
    fn drop(&mut self) {
        self.control.close();
    }
}
