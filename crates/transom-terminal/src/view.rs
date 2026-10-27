use crate::audit::AuditSink;
use crate::block::BlockLayout;
use crate::engine::TerminalEngine;
use crate::event::{HistoryProvider, TerminalEvent};
use crate::input::{MouseButtonCode, encode_mouse, focus_bytes, keystroke_to_bytes, paste_bytes};
use crate::kitty_graphics::GraphicsPlacement;
use crate::osc::{CommandTracker, OscEvent};
use crate::record::RecorderHandle;
use crate::renderer::{PaintRequest, TerminalRenderer};
use alacritty_terminal::event::{Event, WindowSize};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point as AlacPoint, Side};
use alacritty_terminal::selection::SelectionType;
use alacritty_terminal::term::TermMode;
use alacritty_terminal::vte::ansi::{CursorShape, Rgb as AnsiRgb};
use gpui_kit::assets::IconName;
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::InputState;
use gpui_kit::component::menu::{ContextMenuExt, PopupMenuItem};
use gpui_kit::*;
use parking_lot::Mutex;
use std::io::{Read, Write};
use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Instant;
use transom_core::{
    BlockViewSettings, Rgb as PaletteColor, RightClickBehavior, TerminalControlHandle,
    TerminalInteraction, TerminalIo, TerminalProfile, TerminalSize,
};

const CONTEXT: &str = "Terminal";

/// 检索刻度带的分桶高度（像素）：命中再多也按像素合并，避免一帧画出上千个矩形。
const SEARCH_TICK_PIXELS: f32 = 2.0;

actions!(transom_terminal, [SendTab, SendBacktab]);

pub type KeyFilter = Arc<dyn Fn(&Keystroke) -> bool + Send + Sync>;

#[derive(Clone, Copy, Default)]
pub(crate) struct LayoutMetrics {
    pub(crate) origin: Point<Pixels>,
    pub(crate) cell_width: Pixels,
    pub(crate) cell_height: Pixels,
    pub(crate) cols: usize,
    pub(crate) rows: usize,
    /// 检索刻度带的矩形；仅在检索栏打开的一帧里有值，供鼠标跳转命中测试。
    pub(crate) search_strip: Option<Bounds<Pixels>>,
    /// 终端画布在本帧占据的矩形；悬浮操作胶囊按它定位。
    pub(crate) bounds: Option<Bounds<Pixels>>,
}

struct PreparedTerminal {
    /// 本帧可用的检索绘制数据；`None` 表示无检索或快照已失效。
    search: Option<crate::search::SearchPaint>,
    /// 本帧需要重建行缓存的损伤集合。
    damage: crate::renderer::DamageInfo,
    /// 本帧的语义块折叠映射；`None` 表示经典连续流。
    block_layout: Option<Arc<BlockLayout>>,
    /// 本帧要绘制的 Kitty 图像放置。
    graphics: Vec<GraphicsPlacement>,
}

pub struct TerminalView {
    pub(crate) engine: Arc<Mutex<TerminalEngine>>,
    pub(crate) metrics: Arc<Mutex<LayoutMetrics>>,
    /// 长生命周期渲染器：字体度量、调色板预计算表与编译后的高亮规则都缓存于此。
    renderer: Arc<parking_lot::RwLock<TerminalRenderer>>,
    /// 外观变更脏标记：置位后由下一帧 prepaint 持窗口文本系统重建渲染器。
    renderer_dirty: Arc<AtomicBool>,
    /// 每行绘制产物缓存：脏行重建、帧间复用，见 `renderer::RowVisual`。
    row_cache: Arc<Mutex<Vec<crate::renderer::RowVisual>>>,
    profile: TerminalProfile,
    interaction: TerminalInteraction,
    focus_handle: FocusHandle,
    input_tx: flume::Sender<Vec<u8>>,
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
    /// 最近一帧的折叠行映射，鼠标命中与悬浮胶囊按它定位。
    block_layout: Arc<Mutex<Option<Arc<BlockLayout>>>>,
    /// 当前悬浮的命令块与所在可见行；`None` 表示不在块上。
    hover_block: Option<(u64, usize)>,
    _reader_task: Task<()>,
    _blink_task: Task<()>,
    _focus_subscriptions: Vec<Subscription>,
    recorder: RecorderHandle,
    trigger_engine: Arc<Mutex<crate::trigger::TriggerEngine>>,
}

impl TerminalView {
    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        io: TerminalIo,
        profile: TerminalProfile,
        interaction: TerminalInteraction,
        key_filter: Option<KeyFilter>,
        audit_path: Option<std::path::PathBuf>,
    ) -> Self {
        cx.bind_keys([
            KeyBinding::new("tab", SendTab, Some(CONTEXT)),
            KeyBinding::new("shift-tab", SendBacktab, Some(CONTEXT)),
        ]);

        let focus_handle = cx.focus_handle().tab_stop(false);
        let (event_tx, event_rx) = flume::bounded(256);
        let writer_exit_tx = event_tx.clone();
        let exit_tx = event_tx.clone();
        let engine = Arc::new(Mutex::new(TerminalEngine::new(
            TerminalSize::new(80, 24),
            &profile,
            event_tx,
        )));
        let metrics = Arc::new(Mutex::new(LayoutMetrics::default()));

        let recorder = RecorderHandle::new();
        let recorder_for_reader = recorder.clone();
        let (bytes_tx, bytes_rx) = flume::bounded::<Vec<u8>>(128);
        let mut reader = io.reader;
        // 线程 spawn 失败属资源耗尽类可恢复异常：不 panic 整个应用，按"终端已
        // 退出"降级，用户仍能关闭标签页或重连。
        let reader_spawn = thread::Builder::new()
            .name("transom-terminal-reader".into())
            .spawn(move || {
                // 审计落盘挂在这里：本线程已经是专用 OS 线程，同步写盘不会
                // 影响 GPUI 主循环；文件也在此刻才创建，避免打开会话的UI路径上做 IO。
                let mut audit = audit_path.map(AuditSink::new);
                let mut buffer = [0u8; 8192];
                loop {
                    match reader.read(&mut buffer) {
                        Ok(0) => break,
                        Ok(count) => {
                            let slice = &buffer[..count];
                            if let Some(audit) = audit.as_mut() {
                                audit.record(slice);
                            }
                            recorder_for_reader.record_output(slice);
                            if bytes_tx.send(slice.to_vec()).is_err() {
                                break;
                            }
                        }
                        // EINTR 是被信号打断的可重试错误：误判为退出会让终端凭空断开。
                        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(error) => {
                            log::warn!("读取终端输出失败，按已退出处理，原因: {error}");
                            break;
                        }
                    }
                }
                // 退出时刷完缓冲（Drop 也会做，显式调用让"最后一秒不丢"可读）。
                if let Some(audit) = audit.as_mut() {
                    audit.finish();
                }
            });
        if reader_spawn.is_err() {
            log::error!("创建终端读取线程失败，该窗格按已退出降级处理");
            let _ = exit_tx.try_send(Event::Exit);
        }

        // 有界输入通道：深度 256 覆盖人速按键与整批粘贴（每条消息一批字节），
        // 写入线程卡死时通过 try_send 快速失败丢弃而不是无限积压内存。
        let (input_tx, input_rx) = flume::bounded::<Vec<u8>>(256);
        let mut writer = io.writer;
        let writer_spawn = thread::Builder::new()
            .name("transom-terminal-writer".into())
            .spawn(move || {
                while let Ok(bytes) = input_rx.recv() {
                    // write_all 内部已对 Interrupted 自动重试，这里只需兜底真实故障。
                    if let Err(error) = writer.write_all(&bytes) {
                        log::warn!("写入终端输入失败，按已退出处理，原因: {error}");
                        let _ = writer_exit_tx.try_send(Event::Exit);
                        break;
                    }
                    let _ = writer.flush();
                }
            });
        if writer_spawn.is_err() {
            log::error!("创建终端写入线程失败，该窗格按已退出降级处理");
            let _ = exit_tx.try_send(Event::Exit);
        }

        let trigger_engine = Arc::new(Mutex::new(crate::trigger::TriggerEngine::default()));
        let trigger_engine_for_reader = trigger_engine.clone();

        let control = io.control;
        let reader_task = cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            while let Ok(bytes) = bytes_rx.recv_async().await {
                let matches = trigger_engine_for_reader.lock().process_bytes(&bytes);
                let updated = this.update(cx, |view, cx| {
                    view.engine.lock().process(&bytes);
                    for m in matches {
                        match &m.action {
                            transom_core::TriggerAction::AutoResponse(reply) => {
                                let mut to_send = reply.clone();
                                if !to_send.ends_with('\r') && !to_send.ends_with('\n') {
                                    to_send.push('\r');
                                }
                                view.write_input(to_send.as_bytes());
                            }
                            transom_core::TriggerAction::Bell => {
                                cx.emit(TerminalEvent::Bell);
                            }
                            transom_core::TriggerAction::Notification(body) => {
                                cx.emit(TerminalEvent::DesktopNotification {
                                    title: m.name.clone(),
                                    body: body.clone(),
                                });
                            }
                            _ => {}
                        }
                        cx.emit(TerminalEvent::TriggerFired {
                            rule_name: m.name,
                            action: m.action,
                        });
                    }
                    cx.notify();
                });
                if updated.is_err() {
                    return;
                }
            }
            let _ = this.update(cx, |view, cx| view.mark_exited(cx));
        });

        let on_focus = cx.on_focus(&focus_handle, window, |view, _, cx| {
            let mode = view.engine.lock().mode();
            if let Some(bytes) = focus_bytes(mode, true) {
                view.write_input(&bytes);
            }
            view.cursor_blink_on = true;
            cx.notify();
        });
        let on_blur = cx.on_blur(&focus_handle, window, |view, _, cx| {
            let mode = view.engine.lock().mode();
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

        // 渲染器为长生命周期缓存：字体度量、预计算调色板与编译后的高亮规则
        // 只依赖 profile，不随帧内容变化。外观变更经脏标记在下一帧 prepaint
        // （持有窗口文本系统）重建，避免把文本整形探测与规则编译放进每帧热路径。
        let renderer = Arc::new(parking_lot::RwLock::new(TerminalRenderer::new(
            &profile, window, cx,
        )));
        let renderer_dirty = Arc::new(AtomicBool::new(false));
        let row_cache = Arc::new(Mutex::new(Vec::new()));

        Self {
            engine,
            metrics,
            renderer,
            renderer_dirty,
            row_cache,
            profile,
            interaction,
            focus_handle,
            input_tx,
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
            history_provider: None,
            input_line: String::new(),
            suggestions: Vec::new(),
            suggestion_index: 0,
            search_open: false,
            search_input: None,
            search_regex_mode: false,
            _search_subscription: None,
            block_view: None,
            block_layout: Arc::new(Mutex::new(None)),
            hover_block: None,
            _reader_task: reader_task,
            _blink_task: blink_task,
            _focus_subscriptions: vec![on_focus, on_blur],
            recorder,
            trigger_engine,
        }
    }

    /// 从设置重载当前终端的触发器规则。
    pub fn reload_triggers(&mut self, settings: &transom_core::TriggerSettings) {
        self.trigger_engine.lock().reload(settings);
    }

    /// 应用语义块视图配置；`None` 或 `enabled == false` 时回到经典连续流。
    ///
    /// 热切换只改下一帧的行映射与自动折叠阈值，不重建会话、不清屏，
    /// 也不影响已建立的选择与历史缓冲。
    pub fn set_block_view(&mut self, settings: Option<BlockViewSettings>, cx: &mut Context<Self>) {
        if let Some(settings) = settings {
            self.engine
                .lock()
                .set_block_auto_collapse(settings.auto_collapse_rows);
        }
        self.block_view = settings;
        if !self.block_view.is_some_and(|settings| settings.enabled) {
            self.hover_block = None;
            *self.block_layout.lock() = None;
        }
        cx.notify();
    }

    /// 运行时一键切换语义块/经典连续流；返回切换后是否启用块视图。
    pub fn toggle_block_view(&mut self, cx: &mut Context<Self>) -> bool {
        let mut settings = self.block_view.unwrap_or_default();
        settings.enabled = !settings.enabled;
        let enabled = settings.enabled;
        self.set_block_view(Some(settings), cx);
        enabled
    }

    pub fn block_view_enabled(&self) -> bool {
        self.block_view.is_some_and(|settings| settings.enabled)
    }

    /// 切换指定命令块的折叠状态（横幅点击与悬浮胶囊共用）。
    pub fn toggle_block(&mut self, id: u64, cx: &mut Context<Self>) {
        if self.engine.lock().toggle_block(id) {
            cx.notify();
        }
    }

    fn copy_block_output(&mut self, id: u64, cx: &mut Context<Self>) {
        let text = self.engine.lock().block_output_text(id);
        if let Some(text) = text.filter(|text| !text.is_empty()) {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            cx.emit(TerminalEvent::ClipboardStored);
        }
    }

    fn copy_block_markdown(&mut self, id: u64, cx: &mut Context<Self>) {
        let text = self.engine.lock().block_markdown(id);
        if let Some(text) = text {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            cx.emit(TerminalEvent::ClipboardStored);
        }
    }

    /// 重新执行命令块：把命令写进当前窗格并回车。
    fn rerun_block(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(command) = self.engine.lock().block_command_text(id) else {
            return;
        };
        if command.trim().is_empty() {
            return;
        }
        let mut bytes = command.into_bytes();
        bytes.push(b'\r');
        if self.write_input(&bytes) {
            self.engine.lock().scroll_to_bottom();
            self.input_line.clear();
            self.clear_suggestions();
            cx.notify();
        }
    }

    fn tick_cursor_blink(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let blinking = self.focus_handle.is_focused(window) && self.profile.cursor_blink && {
            let engine = self.engine.lock();
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
        // 背景透明度参与渲染：折算成背景色的 alpha，透出下层应用背景。
        self.profile.background_transparency = profile.background_transparency;
        self.renderer_dirty.store(true, Ordering::Release);
        cx.notify();
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

    /// 获取录制器句柄。
    pub fn recorder(&self) -> &RecorderHandle {
        &self.recorder
    }

    /// 查询当前窗格是否正在录制。
    pub fn is_recording(&self) -> bool {
        self.recorder.is_recording()
    }

    /// 开始录制当前终端会话到 `.cast` 文件。
    pub fn start_recording(
        &mut self,
        path: std::path::PathBuf,
        title: Option<String>,
    ) -> std::io::Result<()> {
        let term = self.engine.lock();
        let cols = term.term().grid().columns();
        let rows = term.term().grid().screen_lines();
        drop(term);
        self.recorder.start(path, cols as u16, rows as u16, title)
    }

    /// 停止录制并返回文件路径。
    pub fn stop_recording(&mut self) -> Option<std::path::PathBuf> {
        self.recorder.stop()
    }

    /// 将程序化文本写入终端输入通道。
    ///
    /// 与键盘输入复用同一条有界队列，保证初始化命令按调用顺序到达 Shell；
    /// 返回 `false` 表示终端写入线程已退出（连接已断开）。
    pub fn send_text(&mut self, text: &str, cx: &mut Context<Self>) -> bool {
        let sent = self.write_input(text.as_bytes());
        if sent {
            self.engine.lock().scroll_to_bottom();
            cx.notify();
        }
        sent
    }

    /// 按粘贴语义把一段文本写入终端输入通道，可选在末尾补回车执行。
    ///
    /// 多行脚本使用终端当前的括号粘贴模式：远端 shell 会把整段文本当作一次
    /// 粘贴，而不是每读到一行就提前执行；对端未开启括号粘贴时退回原始字节。
    /// 与键盘输入复用同一条有界队列，返回 `false` 表示终端写入线程已退出。
    pub fn send_paste(&mut self, text: &str, execute: bool, cx: &mut Context<Self>) -> bool {
        let mode = self.engine.lock().mode();
        let mut bytes = paste_bytes(mode, text);
        if execute {
            bytes.push(b'\r');
        }
        let sent = self.write_input(&bytes);
        if sent {
            self.engine.lock().scroll_to_bottom();
            cx.notify();
        }
        sent
    }

    fn mark_exited(&mut self, cx: &mut Context<Self>) {
        if !self.exited {
            self.exited = true;
            cx.emit(TerminalEvent::Exit);
        }
    }

    fn copy_selection(&mut self, cx: &mut Context<Self>) -> bool {
        let text = self.engine.lock().selection_text();
        if let Some(text) = text.filter(|text| !text.is_empty()) {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            cx.emit(TerminalEvent::ClipboardStored);
            true
        } else {
            false
        }
    }

    fn paste_from_clipboard(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return false;
        };
        let mode = self.engine.lock().mode();
        let bytes = paste_bytes(mode, &text);
        if self.write_input(&bytes) {
            self.engine.lock().scroll_to_bottom();
            cx.notify();
            true
        } else {
            false
        }
    }

    fn process_events(&mut self, cx: &mut Context<Self>) {
        self.process_osc_events(cx);
        while let Ok(event) = self.event_rx.try_recv() {
            match event {
                Event::Title(title) => cx.emit(TerminalEvent::Title(title)),
                Event::ResetTitle => cx.emit(TerminalEvent::ResetTitle),
                Event::Bell => cx.emit(TerminalEvent::Bell),
                Event::ClipboardStore(_, text) => {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                    cx.emit(TerminalEvent::ClipboardStored);
                }
                Event::ClipboardLoad(_, formatter) => {
                    if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                        let bytes = formatter(&text);
                        self.write_input(bytes.as_bytes());
                    }
                }
                Event::ColorRequest(index, formatter) => {
                    let rgb = self.ansi_rgb(index);
                    let bytes = formatter(rgb);
                    self.write_input(bytes.as_bytes());
                }
                Event::TextAreaSizeRequest(formatter) => {
                    let metrics = *self.metrics.lock();
                    let size = self.engine.lock().size();
                    let window_size = WindowSize {
                        num_lines: size.rows,
                        num_cols: size.cols,
                        cell_width: f32::from(metrics.cell_width) as u16,
                        cell_height: f32::from(metrics.cell_height) as u16,
                    };
                    let bytes = formatter(window_size);
                    self.write_input(bytes.as_bytes());
                }
                Event::PtyWrite(text) => {
                    self.write_input(text.as_bytes());
                }
                Event::Exit | Event::ChildExit(_) => self.mark_exited(cx),
                Event::CursorBlinkingChange | Event::MouseCursorDirty | Event::Wakeup => {}
            }
        }
    }

    /// 把旁路扫到的 OSC 序列翻成对外的终端事件。
    ///
    /// 单独成函数是因为它和 alacritty 的事件通道无关：那些走 `event_rx`，这些挂在引擎里，
    /// 两者都必须在同一渲染帧里排空，否则通知会滞后到下一次输出。
    fn process_osc_events(&mut self, cx: &mut Context<Self>) {
        let events = self.engine.lock().take_osc_events();
        if events.is_empty() {
            return;
        }
        let now = Instant::now();
        for event in events {
            match event {
                OscEvent::Notify { title, body } => {
                    cx.emit(TerminalEvent::DesktopNotification { title, body });
                }
                OscEvent::CommandRecorded { command } => {
                    cx.emit(TerminalEvent::CommandRecorded { command });
                }
                event => {
                    if let Some(tracked) = self.command_tracker.on_event(&event, now) {
                        cx.emit(TerminalEvent::CommandFinished {
                            exit_code: tracked.exit_code,
                            duration_ms: tracked.duration_ms,
                        });
                    }
                }
            }
        }
    }

    fn ansi_rgb(&self, index: usize) -> AnsiRgb {
        let palette = &self.profile.palette;
        let engine = self.engine.lock();
        let colors = engine.term().colors();
        if index < 256
            && let Some(rgb) = colors[index]
        {
            return AnsiRgb {
                r: rgb.r,
                g: rgb.g,
                b: rgb.b,
            };
        }
        let color = if index < 16 {
            palette.ansi[index]
        } else if index == 256 {
            palette.foreground
        } else if index == 257 {
            palette.background
        } else if index == 258 {
            palette.cursor
        } else if index < 232 {
            let index = index - 16;
            let r = (index / 36) as u8;
            let g = ((index % 36) / 6) as u8;
            let b = (index % 6) as u8;
            let channel = |value: u8| if value == 0 { 0 } else { value * 40 + 55 };
            PaletteColor::new(channel(r), channel(g), channel(b))
        } else {
            // 灰度坡道只有 24 级（232..=255）。index 来自 OSC 序列的外部输入，
            // 可能超过合法区间：先钳制到最亮一级再乘加，杜绝 u8 截断回绕与溢出。
            // min 保证 shade ≤ 23，故 as u8 与 8 + shade * 10（≤ 238）都不会溢出。
            let shade = index.saturating_sub(232).min(23) as u8;
            let level = 8 + shade * 10;
            PaletteColor::new(level, level, level)
        };
        AnsiRgb {
            r: color.r,
            g: color.g,
            b: color.b,
        }
    }

    fn hit_test_screen_cell(&self, position: Point<Pixels>) -> Option<(usize, usize)> {
        let metrics = *self.metrics.lock();
        if metrics.cols == 0
            || metrics.rows == 0
            || metrics.cell_width <= px(0.0)
            || metrics.cell_height <= px(0.0)
        {
            return None;
        }
        let x = position.x - metrics.origin.x;
        let y = position.y - metrics.origin.y;
        if x < px(0.0) || y < px(0.0) {
            return None;
        }
        let col = (f32::from(x) / f32::from(metrics.cell_width)).floor() as usize;
        let row = (f32::from(y) / f32::from(metrics.cell_height)).floor() as usize;
        if col >= metrics.cols || row >= metrics.rows {
            return None;
        }
        Some((row, col))
    }

    fn hit_test_grid_point(&self, position: Point<Pixels>) -> Option<AlacPoint> {
        let (row, col) = self.hit_test_screen_cell(position)?;
        // 折叠模式下可见行与网格行不再一一对应：命中必须查行映射，
        // 否则选择会落到被隐藏的输出区或错位的行上。
        if let Some(layout) = self.block_layout.lock().clone() {
            return layout
                .line_at(row)
                .map(|line| AlacPoint::new(line, Column(col)));
        }
        let offset = self.engine.lock().display_offset() as i32;
        Some(AlacPoint::new(Line(row as i32 - offset), Column(col)))
    }

    /// 命中折叠横幅时返回块 ID。
    fn banner_at_position(&self, position: Point<Pixels>) -> Option<u64> {
        let (row, _) = self.hit_test_screen_cell(position)?;
        self.block_layout.lock().as_ref()?.banner_at(row)
    }

    /// 依据指针位置更新悬浮命令块；只在跨越块边界时请求重绘。
    fn update_hover_block(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let next = self.hover_block_at_position(position);
        if next != self.hover_block {
            self.hover_block = next;
            cx.notify();
        }
    }

    fn hover_block_at_position(&self, position: Point<Pixels>) -> Option<(u64, usize)> {
        let (row, _) = self.hit_test_screen_cell(position)?;
        let layout = self.block_layout.lock().clone()?;
        if let Some(id) = layout.banner_at(row) {
            return Some((id, row));
        }
        let line = layout.line_at(row)?;
        let engine = self.engine.lock();
        let absolute = engine.grid().history_size() as i64 + i64::from(line.0);
        engine
            .command_block_at(absolute)
            .map(|block| (block.id, row))
    }

    /// 悬浮操作胶囊：复制输出、复制 Markdown、重跑与折叠切换。
    fn render_block_actions(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (block_id, row) = self.hover_block?;
        let metrics = *self.metrics.lock();
        let bounds = metrics.bounds?;
        if metrics.cell_height <= px(0.0) {
            return None;
        }
        let top = metrics.origin.y - bounds.origin.y + metrics.cell_height * row as f32;
        let collapsed = self
            .engine
            .lock()
            .command_block(block_id)
            .is_some_and(|block| block.collapsed);
        let fold_icon = if collapsed {
            IconName::UnfoldVertical
        } else {
            IconName::FoldVertical
        };
        let fold_id = ElementId::Name(format!("block-fold-{block_id}").into());
        let copy_id = ElementId::Name(format!("block-copy-{block_id}").into());
        let markdown_id = ElementId::Name(format!("block-markdown-{block_id}").into());
        let rerun_id = ElementId::Name(format!("block-rerun-{block_id}").into());
        Some(
            div()
                .id(ElementId::Name(format!("block-actions-{block_id}").into()))
                .absolute()
                .top(top)
                .right(px(8.0))
                .flex()
                .gap_1()
                .child(
                    Button::new(copy_id)
                        .ghost()
                        .small()
                        .icon(IconName::Copy)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.copy_block_output(block_id, cx);
                        })),
                )
                .child(
                    Button::new(markdown_id)
                        .ghost()
                        .small()
                        .icon(IconName::FileText)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.copy_block_markdown(block_id, cx);
                        })),
                )
                .child(
                    Button::new(rerun_id)
                        .ghost()
                        .small()
                        .icon(IconName::Play)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.rerun_block(block_id, cx);
                        })),
                )
                .child(
                    Button::new(fold_id)
                        .ghost()
                        .small()
                        .icon(fold_icon)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.toggle_block(block_id, cx);
                        })),
                )
                .into_any_element(),
        )
    }

    fn send_mouse_event(
        &mut self,
        mode: TermMode,
        button: MouseButtonCode,
        pressed: bool,
        motion: bool,
        position: Point<Pixels>,
    ) {
        let Some((row, col)) = self.hit_test_screen_cell(position) else {
            return;
        };
        if let Some(bytes) = encode_mouse(mode, button, pressed, motion, col, row) {
            self.write_input(&bytes);
        }
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(filter) = &self.key_filter
            && filter(&event.keystroke)
        {
            return;
        }

        if !self.ime_text.is_empty() {
            return;
        }

        let key = event.keystroke.key.to_lowercase();
        let modifiers = event.keystroke.modifiers;
        if (modifiers.platform && key == "c")
            || (modifiers.control && modifiers.shift && key == "c")
        {
            self.copy_selection(cx);
            return;
        }
        if (modifiers.platform && key == "v")
            || (modifiers.control && modifiers.shift && key == "v")
        {
            self.paste_from_clipboard(cx);
            return;
        }

        // Tab is consumed by the terminal context action to avoid focus traversal.
        // 建议浮层打开时先用来补全：这是用户此刻最想要 Tab 干的事。
        if key == "tab" {
            self.accept_suggestion(cx);
            return;
        }

        // 普通文本与空格输入由 GPUI 的 EntityInputHandler::replace_text_in_range 处理（支持标准字符与 IME 输入），
        // 避免在 on_key_down 与 replace_text_in_range 中双重写入终端。
        if !modifiers.control
            && !modifiers.alt
            && !modifiers.platform
            && !is_control_or_function_key(&key)
        {
            return;
        }

        let mode = self.engine.lock().mode();
        if let Some(bytes) = keystroke_to_bytes(&event.keystroke, mode)
            && self.write_input(&bytes)
        {
            self.engine.lock().scroll_to_bottom();
            // 回车结算一行、退格删一格，其余控制键（方向键、Ctrl-R、Esc 序列）
            // 都可能让 shell 自己改这一行，本地跟踪只能作废。
            match key.as_str() {
                "enter" => {
                    self.input_line.clear();
                    self.clear_suggestions();
                }
                "backspace" => {
                    self.input_line.pop();
                    self.note_input_changed(false, cx);
                }
                _ => {
                    self.input_line.clear();
                    self.clear_suggestions();
                }
            }
            cx.notify();
        }
    }

    fn on_action_send_tab(&mut self, _: &SendTab, _: &mut Window, cx: &mut Context<Self>) {
        if self.write_input(b"\t") {
            self.engine.lock().scroll_to_bottom();
            cx.notify();
        }
    }

    fn on_action_send_backtab(&mut self, _: &SendBacktab, _: &mut Window, cx: &mut Context<Self>) {
        if self.write_input(b"\x1b[Z") {
            self.engine.lock().scroll_to_bottom();
            cx.notify();
        }
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 刻度带点击优先于文本选择与焦点切换：此时输入框必须保住焦点。
        if self.search_strip_click(event.position, window, cx) {
            return;
        }
        window.focus(&self.focus_handle, cx);
        if let Some(block_id) = self.banner_at_position(event.position) {
            self.toggle_block(block_id, cx);
            return;
        }
        let mode = self.engine.lock().mode();

        match event.button {
            MouseButton::Right => {
                // 三档语义：Paste 直接粘贴；Menu / MenuWithPaste 由
                // ContextMenuExt 打开菜单，这里不再消费点击。
                if self.interaction.right_click_behavior() == RightClickBehavior::Paste {
                    self.paste_from_clipboard(cx);
                }
            }
            MouseButton::Left => {
                // Ctrl+点击优先尝试打开 OSC 8 超链接：命中即消费本次点击，
                // 未命中回落到既有的选择/上报行为，不改变其他修饰键语义。
                if event.modifiers.control && self.open_hyperlink_at(event.position) {
                    return;
                }
                // Alt 拖拽 = 矩形列选。它和 Shift 一样压制程序自己的鼠标上报：
                // 需要抠列的场景恰恰发生在 vim/htop 这类接管鼠标的程序里，
                // 若让位给上报，这个功能在最需要它的界面上就是空摆设。
                let rectangular = event.modifiers.alt;
                if mode.intersects(TermMode::MOUSE_MODE) && !event.modifiers.shift && !rectangular {
                    self.send_mouse_event(mode, MouseButtonCode::Left, true, false, event.position);
                    self.mouse_pressed = true;
                    return;
                }
                if let Some(point) = self.hit_test_grid_point(event.position) {
                    // 双击/三击的选词与选行语义只在矩形模式之外生效：Alt 按下时
                    // 用户要的是"这一块"，不是"这个词"。
                    let ty = if rectangular {
                        SelectionType::Block
                    } else {
                        match event.click_count {
                            2 => SelectionType::Semantic,
                            3.. => SelectionType::Lines,
                            _ => SelectionType::Simple,
                        }
                    };
                    self.engine.lock().start_selection(ty, point, Side::Left);
                    self.dragging_selection = true;
                    cx.notify();
                }
            }
            MouseButton::Middle if mode.intersects(TermMode::MOUSE_MODE) => {
                self.send_mouse_event(mode, MouseButtonCode::Middle, true, false, event.position);
            }
            _ => {}
        }
    }

    /// Ctrl+点击打开终端里的 OSC 8 超链接，返回点击是否命中链接。
    ///
    /// 安全约束：只放行 http/https/mailto/ftp 白名单协议，防止终端输出内容
    /// 借 xdg-open 触发任意协议处理器；命中但协议被拒同样视为已消费，
    /// 避免 Ctrl+点击误启选择拖拽。
    fn open_hyperlink_at(&mut self, position: Point<Pixels>) -> bool {
        let Some(point) = self.hit_test_grid_point(position) else {
            return false;
        };
        let Some(uri) = self.engine.lock().hyperlink_at(point) else {
            return false;
        };
        const ALLOWED_PREFIXES: [&str; 4] = ["http://", "https://", "mailto:", "ftp://"];
        if !ALLOWED_PREFIXES
            .iter()
            .any(|prefix| uri.starts_with(prefix))
        {
            log::warn!("超链接协议不在白名单内，已拒绝打开: {uri}");
            return true;
        }
        log::info!("打开终端超链接: {uri}");
        #[cfg(unix)]
        if let Err(error) = std::process::Command::new("xdg-open").arg(&uri).spawn() {
            log::warn!("打开超链接失败: {error}");
        }
        #[cfg(not(unix))]
        log::warn!("当前平台暂不支持打开终端超链接");
        true
    }

    fn on_mouse_up(&mut self, event: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        if event.button != MouseButton::Left {
            return;
        }
        let mode = self.engine.lock().mode();
        if self.mouse_pressed {
            self.send_mouse_event(mode, MouseButtonCode::Left, false, false, event.position);
            self.mouse_pressed = false;
            return;
        }
        if self.dragging_selection {
            if let Some(point) = self.hit_test_grid_point(event.position) {
                self.engine.lock().update_selection(point, Side::Right);
            }
            self.dragging_selection = false;
            if self.interaction.copy_on_select {
                self.copy_selection(cx);
            }
            cx.notify();
        }
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        let mode = self.engine.lock().mode();
        if self.mouse_pressed && mode.intersects(TermMode::MOUSE_MODE) {
            let wants_motion = mode.contains(TermMode::MOUSE_MOTION)
                || (mode.contains(TermMode::MOUSE_DRAG) && event.dragging());
            if wants_motion {
                self.send_mouse_event(mode, MouseButtonCode::Left, true, true, event.position);
            }
            return;
        }
        if self.dragging_selection && event.dragging() {
            // 指针在视口外（上方历史区）时 hit_test 落空，选区更新交给
            // 边缘自动滚动后的重映射完成
            if let Some(point) = self.hit_test_grid_point(event.position) {
                self.engine.lock().update_selection(point, Side::Right);
            }
            self.edge_scroll_during_drag(event.position, cx);
            cx.notify();
        }
        self.update_hover_block(event.position, cx);
    }

    /// 选区拖拽到视口上下边缘时的自动滚动。
    ///
    /// 设计原理（Why）：拖拽选择跨屏内容时指针停在边缘即应滚动，否则选区
    /// 被视口截断。边缘带约 1.5 行，每个指针事件滚 1 行（与指针采样同频，
    /// 无需独立定时器）；滚动后把指针位置重新映射进网格继续扩展选区，
    /// 否则选区停在滚动前的边界上不再生长。
    /// 代价与局限：速率与指针事件频率挂钩，极快甩动边缘时滚动偏快。
    fn edge_scroll_during_drag(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let metrics = *self.metrics.lock();
        let cell_height = f32::from(metrics.cell_height);
        if cell_height <= 0.0 {
            return;
        }
        let y = f32::from(position.y);
        let top = f32::from(metrics.origin.y);
        let edge = cell_height * 1.5;
        // 视口符号约定与滚轮一致：正数向上翻历史，负数向下回底部
        let delta = if y < top + edge {
            1
        } else if y > top + cell_height * metrics.rows as f32 - edge {
            -1
        } else {
            return;
        };
        self.engine.lock().scroll_display(Scroll::Delta(delta));
        if let Some(point) = self.hit_test_grid_point(position) {
            self.engine.lock().update_selection(point, Side::Right);
        }
        cx.notify();
    }

    fn on_scroll(&mut self, event: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        let lines = match event.delta {
            ScrollDelta::Lines(delta) => delta.y.round() as i32,
            ScrollDelta::Pixels(delta) => {
                let metrics = *self.metrics.lock();
                let height = f32::from(metrics.cell_height);
                if height <= 0.0 {
                    0
                } else {
                    self.scroll_pixel_remainder += f32::from(delta.y) / height;
                    let lines = self.scroll_pixel_remainder.round() as i32;
                    self.scroll_pixel_remainder -= lines as f32;
                    lines
                }
            }
        };
        if lines == 0 {
            return;
        }

        let mode = self.engine.lock().mode();
        if mode.intersects(TermMode::MOUSE_MODE) && !event.modifiers.shift {
            let button = if lines > 0 {
                MouseButtonCode::WheelUp
            } else {
                MouseButtonCode::WheelDown
            };
            for _ in 0..lines.abs().min(5) {
                self.send_mouse_event(mode, button, true, false, event.position);
            }
        } else if mode.contains(TermMode::ALT_SCREEN) && self.profile.alternate_scroll {
            // 备屏没有可滚动的历史缓冲（scroll_display 是空操作），滚轮
            // 转换为方向键发给全屏程序，与主流终端的 alternate scroll 语义一致。
            let sequence: &[u8] = if lines > 0 { b"\x1b[A" } else { b"\x1b[B" };
            for _ in 0..lines.abs().min(5) {
                self.write_input(sequence);
            }
        } else {
            self.engine.lock().scroll_display(Scroll::Delta(lines));
            cx.notify();
        }
    }
}

impl EventEmitter<TerminalEvent> for TerminalView {}

impl Render for TerminalView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.process_events(cx);

        let engine_prepaint = self.engine.clone();
        let engine_paint = self.engine.clone();
        let metrics_prepaint = self.metrics.clone();
        let renderer_prepaint = self.renderer.clone();
        let renderer_paint = self.renderer.clone();
        let row_cache_paint = self.row_cache.clone();
        let renderer_dirty = self.renderer_dirty.clone();
        let control = self.control.clone();
        let profile = self.profile.clone();
        let focus_handle = self.focus_handle.clone();
        let entity = cx.entity();
        let ime_text = self.ime_text.clone();
        let ime_marked_range = self.ime_marked_range.clone();
        let cursor_blink_on = self.cursor_blink_on;
        let search_open = self.search_open;
        let block_view = self.block_view;
        let block_style = block_view
            .map(|settings| settings.style)
            .unwrap_or_default();
        let block_layout_shared = self.block_layout.clone();
        let right_click = self.interaction.right_click_behavior();

        let menu_entity = cx.entity();
        let layer = div()
            .size_full()
            .overflow_hidden()
            .track_focus(&self.focus_handle)
            .key_context(CONTEXT)
            .on_action(cx.listener(Self::on_action_send_tab))
            .on_action(cx.listener(Self::on_action_send_backtab))
            .on_key_down(cx.listener(Self::on_key_down))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::on_mouse_down))
            .on_mouse_down(MouseButton::Middle, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_scroll_wheel(cx.listener(Self::on_scroll))
            .child(
                canvas(
                    move |bounds, window, cx| {
                        // 外观变更（脏标记）在此重建：只有 prepaint 阶段持有
                        // 窗口文本系统，度量（"M"/"中" 整形探测）必须在这里做。
                        let mut renderer_rebuilt = false;
                        if renderer_dirty.swap(false, Ordering::AcqRel) {
                            *renderer_prepaint.write() =
                                TerminalRenderer::new(&profile, window, cx);
                            renderer_rebuilt = true;
                        }
                        let renderer = renderer_prepaint.read();
                        let (cols, rows) = renderer.grid_size(bounds);
                        let size = TerminalSize::new(cols, rows);
                        let mut engine = engine_prepaint.lock();
                        let mut resized = false;
                        if engine.size() != size {
                            engine.resize(size);
                            resized = true;
                            // 尺寸同步失败会直接导致网格与 PTY 失配（提示符错位），
                            // 因此不再静默忽略，按警告记录并给出排查方向
                            if control.resize(size) {
                                log::debug!(
                                    "终端网格尺寸已更新并同步至 PTY，列数: {}，行数: {}",
                                    size.cols,
                                    size.rows
                                );
                            } else {
                                log::warn!(
                                    "PTY 窗口尺寸同步失败（控制通道已断开），画面可能持续错位，请重连会话，列数: {}，行数: {}",
                                    size.cols,
                                    size.rows
                                );
                            }
                        }
                        // 检索重扫放在尺寸变更之后：缩放会重排折行、让快照作废，
                        // 放在数据流路径里又会把刷屏变成卡顿，所以只在渲染帧里按冷却触发。
                        let search = if search_open {
                            engine.refresh_search(std::time::Instant::now());
                            let tick_rows =
                                (f32::from(bounds.size.height) / SEARCH_TICK_PIXELS).max(1.0)
                                    as usize;
                            engine.search_paint(tick_rows)
                        } else {
                            None
                        };
                        // 损伤集合必须在持锁的准备阶段取走并重置：它是"上次取走
                        // 之后全部变化"的完整窗口。折行重排与外观重建让行号、字体
                        // 度量整体失效，无论内核报了什么都提升为全量。
                        let mut damage = engine.take_damage();
                        if renderer_rebuilt || resized {
                            damage = crate::renderer::DamageInfo::Full;
                        }
                        // 刻度带只有在真的画了刻度时才登记靶区，否则会白白吃掉一列点击。
                        let strip = search
                            .as_ref()
                            .filter(|paint| !paint.ticks.is_empty())
                            .map(|_| TerminalRenderer::search_strip_bounds(bounds));
                        let metrics = LayoutMetrics {
                            origin: Point {
                                x: bounds.origin.x + TerminalRenderer::TERMINAL_PADDING_X,
                                y: bounds.origin.y + TerminalRenderer::TERMINAL_PADDING_Y,
                            },
                            cell_width: renderer.cell_width,
                            cell_height: renderer.cell_height,
                            cols: cols as usize,
                            rows: rows as usize,
                            search_strip: strip,
                            bounds: Some(bounds),
                        };
                        *metrics_prepaint.lock() = metrics;
                        let layout = engine
                            .build_block_layout(block_view.as_ref())
                            .map(Arc::new);
                        *block_layout_shared.lock() = layout.clone();
                        let graphics = engine.graphics_placements();
                        PreparedTerminal {
                            search,
                            damage,
                            block_layout: layout,
                            graphics,
                        }
                    },
                    move |bounds, prepared, window, cx| {
                        window.handle_input(
                            &focus_handle,
                            ElementInputHandler::new(bounds, entity.clone()),
                            cx,
                        );
                        let engine = engine_paint.lock();
                        let renderer = renderer_paint.read();
                        let mut row_cache = row_cache_paint.lock();
                        let ime = if ime_text.is_empty() {
                            None
                        } else {
                            Some((ime_text.as_str(), ime_marked_range.as_ref()))
                        };
                        renderer.paint(
                            PaintRequest {
                                bounds,
                                term: engine.term(),
                                ime,
                                blink_visible: cursor_blink_on,
                                search: prepared.search.as_ref(),
                                block_layout: prepared.block_layout.as_deref(),
                                block_style,
                                graphics: &prepared.graphics,
                            },
                            &mut row_cache,
                            &prepared.damage,
                            window,
                            cx,
                        );
                    },
                )
                .size_full(),
            );
        // 只有菜单档位才挂 ContextMenuExt：Paste 档位右键必须直接粘贴，
        // 挂上菜单会让一次右键同时粘贴并弹出菜单。
        let layer: AnyElement = if right_click == RightClickBehavior::Paste {
            layer.into_any_element()
        } else {
            let menu_entity = menu_entity.clone();
            layer
                .context_menu(move |menu, _window, _cx| {
                    let mut menu = menu;
                    if right_click == RightClickBehavior::MenuWithPaste {
                        let app = menu_entity.clone();
                        menu = menu
                            .item(
                                PopupMenuItem::new("粘贴")
                                    .icon(IconName::ClipboardPaste)
                                    .on_click(move |_, _, cx| {
                                        app.update(cx, |view, cx| {
                                            view.paste_from_clipboard(cx);
                                        });
                                    }),
                            )
                            .item(PopupMenuItem::separator());
                    }
                    let app = menu_entity.clone();
                    menu.item(
                        PopupMenuItem::new("切换语义块视图 / 经典连续流")
                            .icon(IconName::ListTree)
                            .on_click(move |_, _, cx| {
                                app.update(cx, |view, cx| {
                                    view.toggle_block_view(cx);
                                });
                            }),
                    )
                })
                .into_any_element()
        };

        let search_bar = self.render_search_bar(cx);
        let block_actions = self.render_block_actions(cx);
        // 拖放高亮用主色调半透明蒙色：不改布局、不依赖主题里的具体色值，
        // 深浅色终端下都能看清"松手就落在这里"。
        let drop_tint = cx.theme().primary.opacity(0.12);
        let entity = cx.entity().clone();
        // 检索栏与终端层平级：放在终端层子树里会让输入框的按键沿焦点路径冒泡到
        // `on_key_down`，敲搜索词的同时也把字符送进了 Shell。
        div()
            .size_full()
            .relative()
            .overflow_hidden()
            .on_action(cx.listener(Self::on_search_enter))
            .on_action(cx.listener(Self::on_search_escape))
            // 外部文件拖放：命中区域就是网格本身，视图只投递事件，
            // 要不要接收、传到哪儿由应用层按会话类型决定。
            .drag_over::<ExternalPaths>(move |style, _, _, _| style.bg(drop_tint))
            .on_drop(move |paths: &ExternalPaths, _window, cx| {
                let dropped = paths.paths().to_vec();
                if dropped.is_empty() {
                    return;
                }
                entity.update(cx, |_, cx| cx.emit(TerminalEvent::FilesDropped(dropped)));
            })
            .child(layer)
            .children(block_actions)
            .children(self.render_suggestion_bar(cx))
            .children(search_bar)
    }
}

impl Drop for TerminalView {
    fn drop(&mut self) {
        self.control.close();
    }
}

fn is_control_or_function_key(key: &str) -> bool {
    matches!(
        key,
        "enter"
            | "escape"
            | "backspace"
            | "tab"
            | "up"
            | "down"
            | "right"
            | "left"
            | "home"
            | "end"
            | "pageup"
            | "pagedown"
            | "insert"
            | "delete"
            | "f1"
            | "f2"
            | "f3"
            | "f4"
            | "f5"
            | "f6"
            | "f7"
            | "f8"
            | "f9"
            | "f10"
            | "f11"
            | "f12"
    )
}

#[cfg(test)]
mod tests {
    use crate::view_ime::ime_column_to_utf16_index;

    /// IME 定位折算：ASCII 单格、非 ASCII 双格近似，越界收敛到端点。
    #[test]
    fn ime_column_maps_cells_to_utf16_index() {
        // "ab😀" 的格占用为 1+1+2；UTF-16 长度为 4（表情符号占两个单元）。
        assert_eq!(ime_column_to_utf16_index("ab😀", 0), 0);
        assert_eq!(ime_column_to_utf16_index("ab😀", 1), 1);
        // 第 2 格已进入表情符号的区间，返回其起点下标 2
        assert_eq!(ime_column_to_utf16_index("ab😀", 2), 2);
        // 越过组合文本末尾收敛到全长
        assert_eq!(ime_column_to_utf16_index("ab😀", 9), 4);
        assert_eq!(ime_column_to_utf16_index("", 3), 0);
    }
}
