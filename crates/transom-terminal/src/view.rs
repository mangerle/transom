use crate::engine::TerminalEngine;
use crate::event::TerminalEvent;
use crate::input::{MouseButtonCode, encode_mouse, focus_bytes, keystroke_to_bytes, paste_bytes};
use crate::osc::{CommandTracker, OscEvent};
use crate::renderer::{PaintRequest, TerminalRenderer};
use alacritty_terminal::event::{Event, WindowSize};
use alacritty_terminal::grid::Scroll;
use alacritty_terminal::index::{Column, Line, Point as AlacPoint, Side};
use alacritty_terminal::selection::SelectionType;
use alacritty_terminal::term::TermMode;
use alacritty_terminal::vte::ansi::{CursorShape, Rgb as AnsiRgb};
use gpui_kit::component::input::InputState;
use gpui_kit::*;
use parking_lot::Mutex;
use std::io::{Read, Write};
use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Instant;
use transom_core::{
    Rgb as PaletteColor, TerminalControlHandle, TerminalInteraction, TerminalIo, TerminalProfile,
    TerminalSize,
};

const CONTEXT: &str = "Terminal";

/// 检索刻度带的分桶高度（像素）：命中再多也按像素合并，避免一帧画出上千个矩形。
const SEARCH_TICK_PIXELS: f32 = 2.0;

actions!(transom_terminal, [SendTab, SendBacktab]);

pub type KeyFilter = Arc<dyn Fn(&Keystroke) -> bool + Send + Sync>;

#[derive(Clone, Copy, Default)]
pub(crate) struct LayoutMetrics {
    origin: Point<Pixels>,
    cell_width: Pixels,
    cell_height: Pixels,
    cols: usize,
    rows: usize,
    /// 检索刻度带的矩形；仅在检索栏打开的一帧里有值，供鼠标跳转命中测试。
    pub(crate) search_strip: Option<Bounds<Pixels>>,
}

struct PreparedTerminal {
    /// 本帧可用的检索绘制数据；`None` 表示无检索或快照已失效。
    search: Option<crate::search::SearchPaint>,
    /// 本帧需要重建行缓存的损伤集合。
    damage: crate::renderer::DamageInfo,
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
    ime_text: String,
    ime_marked_range: Option<Range<usize>>,
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
    /// 是否把查询词当正则表达式解析（与引擎里的快照同步）。
    pub(crate) search_regex_mode: bool,
    /// 检索输入框的事件订阅；随实体存活，收起搜索栏也不释放（下次复用同一输入框）。
    pub(crate) _search_subscription: Option<Subscription>,
    _reader_task: Task<()>,
    _blink_task: Task<()>,
    _focus_subscriptions: Vec<Subscription>,
}

impl TerminalView {
    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        io: TerminalIo,
        profile: TerminalProfile,
        interaction: TerminalInteraction,
        key_filter: Option<KeyFilter>,
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

        let (bytes_tx, bytes_rx) = flume::bounded::<Vec<u8>>(128);
        let mut reader = io.reader;
        // 线程 spawn 失败属资源耗尽类可恢复异常：不 panic 整个应用，按"终端已
        // 退出"降级，用户仍能关闭标签页或重连。
        let reader_spawn = thread::Builder::new()
            .name("transom-terminal-reader".into())
            .spawn(move || {
                let mut buffer = [0u8; 8192];
                loop {
                    match reader.read(&mut buffer) {
                        Ok(0) => break,
                        Ok(count) => {
                            if bytes_tx.send(buffer[..count].to_vec()).is_err() {
                                break;
                            }
                        }
                        // EINTR 是被信号打断的可重试错误：误判为退出会让终端凭空断开。
                        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(error) => {
                            tracing::warn!("读取终端输出失败，按已退出处理，原因: {error}");
                            break;
                        }
                    }
                }
            });
        if reader_spawn.is_err() {
            tracing::error!("创建终端读取线程失败，该窗格按已退出降级处理");
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
                        tracing::warn!("写入终端输入失败，按已退出处理，原因: {error}");
                        let _ = writer_exit_tx.try_send(Event::Exit);
                        break;
                    }
                    let _ = writer.flush();
                }
            });
        if writer_spawn.is_err() {
            tracing::error!("创建终端写入线程失败，该窗格按已退出降级处理");
            let _ = exit_tx.try_send(Event::Exit);
        }

        let control = io.control;
        let reader_task = cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            while let Ok(bytes) = bytes_rx.recv_async().await {
                let updated = this.update(cx, |view, cx| {
                    view.engine.lock().process(&bytes);
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
            search_open: false,
            search_input: None,
            search_regex_mode: false,
            _search_subscription: None,
            _reader_task: reader_task,
            _blink_task: blink_task,
            _focus_subscriptions: vec![on_focus, on_blur],
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
        self.renderer_dirty.store(true, Ordering::Release);
        cx.notify();
    }

    fn write_input(&mut self, bytes: &[u8]) -> bool {
        if bytes.is_empty() {
            return true;
        }
        // 输入瞬间强制点亮光标，避免用户打字时恰好处于熄灭相位。
        self.cursor_blink_on = true;
        match self.input_tx.try_send(bytes.to_vec()) {
            Ok(()) => true,
            // 256 深度对人速按键与整批粘贴绰绰有余，打满只可能是写入线程
            // 卡死：此时丢掉本批输入并继续存活判定，比在 UI 线程阻塞等待好。
            Err(flume::TrySendError::Full(_)) => {
                tracing::warn!("终端输入队列已满，本批输入被丢弃");
                true
            }
            Err(flume::TrySendError::Disconnected(_)) => false,
        }
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
        let offset = self.engine.lock().display_offset() as i32;
        Some(AlacPoint::new(Line(row as i32 - offset), Column(col)))
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
        if key == "tab" {
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
        let mode = self.engine.lock().mode();

        match event.button {
            MouseButton::Right => {
                if self.interaction.right_click_paste && !event.modifiers.shift {
                    self.paste_from_clipboard(cx);
                } else {
                    cx.emit(TerminalEvent::ContextMenuRequested {
                        x: f32::from(event.position.x),
                        y: f32::from(event.position.y),
                    });
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
            tracing::warn!("超链接协议不在白名单内，已拒绝打开: {uri}");
            return true;
        }
        tracing::info!("打开终端超链接: {uri}");
        #[cfg(unix)]
        if let Err(error) = std::process::Command::new("xdg-open").arg(&uri).spawn() {
            tracing::warn!("打开超链接失败: {error}");
        }
        #[cfg(not(unix))]
        tracing::warn!("当前平台暂不支持打开终端超链接");
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
        if self.dragging_selection
            && event.dragging()
            && let Some(point) = self.hit_test_grid_point(event.position)
        {
            self.engine.lock().update_selection(point, Side::Right);
            cx.notify();
        }
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
        } else {
            self.engine.lock().scroll_display(Scroll::Delta(lines));
            cx.notify();
        }
    }
}

impl EntityInputHandler for TerminalView {
    fn text_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let range = self
            .ime_marked_range
            .clone()
            .unwrap_or(0..self.ime_text.encode_utf16().count());
        *adjusted_range = Some(range);
        Some(self.ime_text.clone())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: 0..0,
            reversed: false,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.ime_marked_range.clone()
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.ime_marked_range = None;
        self.ime_text.clear();
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        _range: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.ime_marked_range = None;
        self.ime_text.clear();
        if !text.is_empty() {
            self.write_input(text.as_bytes());
            self.engine.lock().scroll_to_bottom();
        }
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _range: Option<Range<usize>>,
        new_text: &str,
        new_selected_range: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.ime_text = new_text.to_string();
        self.ime_marked_range = if new_text.is_empty() {
            None
        } else {
            new_selected_range.or_else(|| Some(0..new_text.encode_utf16().count()))
        };
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        _element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let metrics = *self.metrics.lock();
        if metrics.cols == 0 || metrics.rows == 0 {
            return None;
        }
        let engine = self.engine.lock();
        let cursor = engine.cursor_point();
        let offset = engine.display_offset() as i32;
        let row = cursor.line.0 + offset;
        if row < 0 || row as usize >= metrics.rows || cursor.column.0 >= metrics.cols {
            return None;
        }
        Some(Bounds {
            origin: Point {
                x: metrics.origin.x + metrics.cell_width * cursor.column.0 as f32,
                y: metrics.origin.y + metrics.cell_height * row as f32,
            },
            size: Size {
                width: metrics.cell_width.max(px(1.0)),
                height: metrics.cell_height.max(px(1.0)),
            },
        })
    }

    fn character_index_for_point(
        &mut self,
        _point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        Some(0)
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
                                tracing::debug!(
                                    cols = size.cols,
                                    rows = size.rows,
                                    "终端网格尺寸已更新并同步至 PTY"
                                );
                            } else {
                                tracing::warn!(
                                    cols = size.cols,
                                    rows = size.rows,
                                    "PTY 窗口尺寸同步失败（控制通道已断开），画面可能持续错位，请重连会话"
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
                        };
                        *metrics_prepaint.lock() = metrics;
                        PreparedTerminal { search, damage }
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

        let search_bar = self.render_search_bar(cx);
        // 检索栏与终端层平级：放在终端层子树里会让输入框的按键沿焦点路径冒泡到
        // `on_key_down`，敲搜索词的同时也把字符送进了 Shell。
        div()
            .size_full()
            .relative()
            .overflow_hidden()
            .on_action(cx.listener(Self::on_search_enter))
            .on_action(cx.listener(Self::on_search_escape))
            .child(layer)
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
