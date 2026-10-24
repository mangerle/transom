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
    renderer: TerminalRenderer,
    /// 本帧可用的检索绘制数据；`None` 表示无检索或快照已失效。
    search: Option<crate::search::SearchPaint>,
}

pub struct TerminalView {
    pub(crate) engine: Arc<Mutex<TerminalEngine>>,
    pub(crate) metrics: Arc<Mutex<LayoutMetrics>>,
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

        let (input_tx, input_rx) = flume::unbounded::<Vec<u8>>();
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

        Self {
            engine,
            metrics,
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
    /// 为什么需要这个入口：渲染器每帧都从 `self.profile` 重建，只改设置里的字体而已经
    /// 打开的窗格毫无反应，等于"设置不生效"。调色板、scrollback 与光标外观不在这里覆盖，
    /// 那些仍归会话自己的 profile，避免全局偏好悄悄吃掉会话级配置。
    /// 网格列数会随字号变化，由下一帧 prepaint 统一走既有的 resize + PTY 同步通路。
    pub fn apply_appearance(&mut self, profile: &TerminalProfile, cx: &mut Context<Self>) {
        self.profile.font_family = profile.font_family.clone();
        self.profile.cjk_font_family = profile.cjk_font_family.clone();
        self.profile.font_size = profile.font_size;
        self.profile.line_height = profile.line_height;
        self.profile.highlight = profile.highlight.clone();
        cx.notify();
    }

    fn write_input(&mut self, bytes: &[u8]) -> bool {
        if bytes.is_empty() {
            return true;
        }
        // 输入瞬间强制点亮光标，避免用户打字时恰好处于熄灭相位。
        self.cursor_blink_on = true;
        self.input_tx.send(bytes.to_vec()).is_ok()
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
                    (f32::from(delta.y) / height).round() as i32
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
                        let renderer = TerminalRenderer::new(&profile, window, cx);
                        let (cols, rows) = renderer.grid_size(bounds);
                        let size = TerminalSize::new(cols, rows);
                        let mut engine = engine_prepaint.lock();
                        if engine.size() != size {
                            engine.resize(size);
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
                        PreparedTerminal { renderer, search }
                    },
                    move |bounds, prepared, window, cx| {
                        window.handle_input(
                            &focus_handle,
                            ElementInputHandler::new(bounds, entity.clone()),
                            cx,
                        );
                        let engine = engine_paint.lock();
                        let ime = if ime_text.is_empty() {
                            None
                        } else {
                            Some((ime_text.as_str(), ime_marked_range.as_ref()))
                        };
                        prepared.renderer.paint(
                            PaintRequest {
                                bounds,
                                term: engine.term(),
                                ime,
                                blink_visible: cursor_blink_on,
                                search: prepared.search.as_ref(),
                            },
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
