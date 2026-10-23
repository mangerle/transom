use crate::engine::TerminalEngine;
use crate::event::TerminalEvent;
use crate::input::{MouseButtonCode, encode_mouse, focus_bytes, keystroke_to_bytes, paste_bytes};
use crate::renderer::TerminalRenderer;
use alacritty_terminal::event::{Event, WindowSize};
use alacritty_terminal::grid::Scroll;
use alacritty_terminal::index::{Column, Line, Point as AlacPoint, Side};
use alacritty_terminal::selection::SelectionType;
use alacritty_terminal::term::TermMode;
use alacritty_terminal::vte::ansi::Rgb as AnsiRgb;
use gpui_kit::*;
use parking_lot::Mutex;
use std::io::{Read, Write};
use std::ops::Range;
use std::sync::Arc;
use std::thread;
use transom_core::{
    Rgb as PaletteColor, TerminalControlHandle, TerminalInteraction, TerminalIo, TerminalProfile,
    TerminalSize,
};

const CONTEXT: &str = "Terminal";

actions!(transom_terminal, [SendTab, SendBacktab]);

pub type KeyFilter = Arc<dyn Fn(&Keystroke) -> bool + Send + Sync>;

#[derive(Clone, Copy, Default)]
struct LayoutMetrics {
    origin: Point<Pixels>,
    cell_width: Pixels,
    cell_height: Pixels,
    cols: usize,
    rows: usize,
}

struct PreparedTerminal {
    renderer: TerminalRenderer,
}

pub struct TerminalView {
    engine: Arc<Mutex<TerminalEngine>>,
    metrics: Arc<Mutex<LayoutMetrics>>,
    profile: TerminalProfile,
    interaction: TerminalInteraction,
    focus_handle: FocusHandle,
    input_tx: flume::Sender<Vec<u8>>,
    control: TerminalControlHandle,
    event_rx: flume::Receiver<Event>,
    key_filter: Option<KeyFilter>,
    ime_text: String,
    ime_marked_range: Option<Range<usize>>,
    dragging_selection: bool,
    mouse_pressed: bool,
    exited: bool,
    _reader_task: Task<()>,
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
        let engine = Arc::new(Mutex::new(TerminalEngine::new(
            TerminalSize::new(80, 24),
            &profile,
            event_tx,
        )));
        let metrics = Arc::new(Mutex::new(LayoutMetrics::default()));

        let (bytes_tx, bytes_rx) = flume::bounded::<Vec<u8>>(128);
        let mut reader = io.reader;
        thread::Builder::new()
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
                        Err(_) => break,
                    }
                }
            })
            .expect("failed to spawn terminal reader");

        let (input_tx, input_rx) = flume::unbounded::<Vec<u8>>();
        let mut writer = io.writer;
        thread::Builder::new()
            .name("transom-terminal-writer".into())
            .spawn(move || {
                while let Ok(bytes) = input_rx.recv() {
                    if writer.write_all(&bytes).is_err() {
                        let _ = writer_exit_tx.try_send(Event::Exit);
                        break;
                    }
                    let _ = writer.flush();
                }
            })
            .expect("failed to spawn terminal writer");

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
            cx.notify();
        });
        let on_blur = cx.on_blur(&focus_handle, window, |view, _, cx| {
            let mode = view.engine.lock().mode();
            if let Some(bytes) = focus_bytes(mode, false) {
                view.write_input(&bytes);
            }
            cx.notify();
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
            _reader_task: reader_task,
            _focus_subscriptions: vec![on_focus, on_blur],
        }
    }

    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus_handle
    }

    fn write_input(&self, bytes: &[u8]) -> bool {
        if bytes.is_empty() {
            return true;
        }
        self.input_tx.send(bytes.to_vec()).is_ok()
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
            let level = 8 + (index.saturating_sub(232) as u8) * 10;
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
        &self,
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
                if mode.intersects(TermMode::MOUSE_MODE) && !event.modifiers.shift {
                    self.send_mouse_event(mode, MouseButtonCode::Left, true, false, event.position);
                    self.mouse_pressed = true;
                    return;
                }
                if let Some(point) = self.hit_test_grid_point(event.position) {
                    let ty = match event.click_count {
                        2 => SelectionType::Semantic,
                        3.. => SelectionType::Lines,
                        _ => SelectionType::Simple,
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

        div()
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
                        {
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
                        }
                        let metrics = LayoutMetrics {
                            origin: Point {
                                x: bounds.origin.x + TerminalRenderer::TERMINAL_PADDING_X,
                                y: bounds.origin.y + TerminalRenderer::TERMINAL_PADDING_Y,
                            },
                            cell_width: renderer.cell_width,
                            cell_height: renderer.cell_height,
                            cols: cols as usize,
                            rows: rows as usize,
                        };
                        *metrics_prepaint.lock() = metrics;
                        PreparedTerminal { renderer }
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
                        prepared
                            .renderer
                            .paint(bounds, engine.term(), window, cx, ime);
                    },
                )
                .size_full(),
            )
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
