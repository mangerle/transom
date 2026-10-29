//! 终端键鼠事件处理、选区拖拽与滚动响应。

use super::{SendBacktab, SendTab, TerminalView};
use crate::input::{
    KeyEventKind, MouseButtonCode, encode_mouse, keystroke_event_to_bytes, tab_bytes,
};
use alacritty_terminal::grid::Scroll;
use alacritty_terminal::index::Side;
use alacritty_terminal::selection::SelectionType;
use alacritty_terminal::term::TermMode;
use gpui_kit::*;
use transom_core::RightClickBehavior;

impl TerminalView {
    pub(super) fn send_mouse_event(
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

    pub(super) fn on_key_down(
        &mut self,
        event: &KeyDownEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(filter) = &self.key_filter
            && filter(&event.keystroke)
        {
            return;
        }

        if !self.ime_text.is_empty() {
            return;
        }

        // 能走到这里就说明这次按键属于终端输入域（应用级快捷键已被 key_filter
        // 截走）。记下它，Shell 集成便不再尝试注入：往半截命令行里写字节会直接
        // 毁掉用户正在敲的那条命令，宁可少一套语义块。
        self.user_typed = true;

        // 自定义映射优先于终端默认编码，也允许覆盖 Ctrl+Shift+C/V；
        // 应用级快捷键已在 key_filter 之前被保留，不会在这里被吃掉。
        if let Some(bytes) = self.custom_key_sequence(&event.keystroke) {
            if self.write_input(&bytes) {
                self.state.engine.lock().scroll_to_bottom();
                self.input_line.clear();
                self.clear_suggestions();
                cx.notify();
            }
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
        // 带修饰键的 Tab（Ctrl+Tab / Ctrl+Shift+Tab）必须继续走向编码器：
        // 在 Kitty 协议下它们是与 Tab 完全不同的按键，吞掉就违背了协商语义。
        if key == "tab"
            && !modifiers.control
            && !modifiers.alt
            && !modifiers.platform
            && !modifiers.shift
        {
            self.accept_suggestion(cx);
            return;
        }

        let mode = self.state.engine.lock().mode();
        // "全部按键按转义序列上报"开启后，普通字符也必须由这里编码；平台随后
        // 会尝试把同一按键当文本提交，必须阻断，否则字符会进终端两次。
        let all_keys = mode.contains(TermMode::REPORT_ALL_KEYS_AS_ESC);

        // 普通文本与空格输入由 GPUI 的 EntityInputHandler::replace_text_in_range 处理（支持标准字符与 IME 输入），
        // 避免在 on_key_down 与 replace_text_in_range 中双重写入终端。
        if !all_keys
            && !modifiers.control
            && !modifiers.alt
            && !modifiers.platform
            && !is_control_or_function_key(&key)
        {
            return;
        }

        let kind = if event.is_held {
            KeyEventKind::Repeat
        } else {
            KeyEventKind::Press
        };
        if let Some(bytes) = keystroke_event_to_bytes(&event.keystroke, mode, kind)
            && self.write_input(&bytes)
        {
            self.state.engine.lock().scroll_to_bottom();
            let typed_text = all_keys
                && !modifiers.control
                && !modifiers.alt
                && !modifiers.platform
                && event.keystroke.key_char.is_some();
            if typed_text {
                // 文本已由 CSI u 携带（可关联文本），本地输入行要同步累积，
                // 否则命令历史建议在 all-keys 模式下的终端里会停更。
                if let Some(text) = event.keystroke.key_char.as_deref() {
                    self.input_line.push_str(text);
                    self.note_input_changed(false, cx);
                }
                cx.stop_propagation();
            } else {
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
            }
            cx.notify();
        }
    }

    /// 释放事件只服务协商了 `REPORT_EVENT_TYPES` 的应用；未协商时直接忽略，
    /// 传统终端没有"松开"这个概念。
    pub(super) fn on_key_up(&mut self, event: &KeyUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        let mode = self.state.engine.lock().mode();
        if !mode.contains(TermMode::REPORT_EVENT_TYPES) {
            return;
        }
        if let Some(bytes) = keystroke_event_to_bytes(&event.keystroke, mode, KeyEventKind::Release)
            && self.write_input(&bytes)
        {
            cx.notify();
        }
    }

    pub(super) fn on_action_send_tab(
        &mut self,
        _: &SendTab,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 与 on_key_down 同源：Tab 也是用户亲自敲进 Shell 的按键。
        self.user_typed = true;
        let mode = self.state.engine.lock().mode();
        if self.write_input(&tab_bytes(mode, false)) {
            self.state.engine.lock().scroll_to_bottom();
            cx.notify();
        }
    }

    pub(super) fn on_action_send_backtab(
        &mut self,
        _: &SendBacktab,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.user_typed = true;
        let mode = self.state.engine.lock().mode();
        if self.write_input(&tab_bytes(mode, true)) {
            self.state.engine.lock().scroll_to_bottom();
            cx.notify();
        }
    }

    pub(super) fn on_mouse_down(
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
        let mode = self.state.engine.lock().mode();

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
                if event.modifiers.control && self.open_hyperlink_at(event.position, cx) {
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
                    self.state
                        .engine
                        .lock()
                        .start_selection(ty, point, Side::Left);
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

    pub(super) fn on_mouse_up(
        &mut self,
        event: &MouseUpEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.button != MouseButton::Left {
            return;
        }
        let mode = self.state.engine.lock().mode();
        if self.mouse_pressed {
            self.send_mouse_event(mode, MouseButtonCode::Left, false, false, event.position);
            self.mouse_pressed = false;
            return;
        }
        if self.dragging_selection {
            if let Some(point) = self.hit_test_grid_point(event.position) {
                self.state
                    .engine
                    .lock()
                    .update_selection(point, Side::Right);
            }
            self.dragging_selection = false;
            if self.interaction.copy_on_select {
                self.copy_selection(cx);
            }
            cx.notify();
        }
    }

    pub(super) fn on_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mode = self.state.engine.lock().mode();
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
                self.state
                    .engine
                    .lock()
                    .update_selection(point, Side::Right);
            }
            self.edge_scroll_during_drag(event.position, cx);
            cx.notify();
        }
        self.update_hover_block(event.position, cx);
        // 拖拽选区时不切手型：此刻用户的操作对象是文本，不是链接。
        if !self.mouse_pressed && !self.dragging_selection {
            self.update_link_cursor(event.position, window);
        }
    }

    /// 选区拖拽到视口上下边缘时的自动滚动。
    ///
    /// 设计原理（Why）：拖拽选择跨屏内容时指针停在边缘即应滚动，否则选区
    /// 被视口截断。边缘带约 1.5 行，每个指针事件滚 1 行（与指针采样同频，
    /// 无需独立定时器）；滚动后把指针位置重新映射进网格继续扩展选区，
    /// 否则选区停在滚动前的边界上不再生长。
    /// 代价与局限：速率与指针事件频率挂钩，极快甩动边缘时滚动偏快。
    pub(super) fn edge_scroll_during_drag(
        &mut self,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let metrics = *self.state.metrics.lock();
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
        self.state
            .engine
            .lock()
            .scroll_display(Scroll::Delta(delta));
        if let Some(point) = self.hit_test_grid_point(position) {
            self.state
                .engine
                .lock()
                .update_selection(point, Side::Right);
        }
        cx.notify();
    }

    pub(super) fn on_scroll(
        &mut self,
        event: &ScrollWheelEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let lines = match event.delta {
            ScrollDelta::Lines(delta) => delta.y.round() as i32,
            ScrollDelta::Pixels(delta) => {
                let metrics = *self.state.metrics.lock();
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

        let mode = self.state.engine.lock().mode();
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
            self.state
                .engine
                .lock()
                .scroll_display(Scroll::Delta(lines));
            cx.notify();
        }
    }
}

/// 把设置里的映射解析成运行期查表结构；坏条目跳过并留日志。
pub(crate) fn build_custom_keys(
    bindings: &[transom_core::TerminalKeyBinding],
) -> Vec<(KeybindingKeystroke, Vec<u8>)> {
    let mut custom = Vec::with_capacity(bindings.len());
    for binding in bindings {
        let keystroke = match Keystroke::parse(&binding.keys) {
            Ok(keystroke) => keystroke,
            Err(error) => {
                log::warn!("自定义按键映射无法解析，已忽略: {error}");
                continue;
            }
        };
        let bytes = match binding.decode_sequence() {
            Ok(bytes) => bytes,
            Err(error) => {
                log::warn!(
                    "自定义按键映射的转义序列非法，已忽略，按键: {}，原因: {error}",
                    binding.keys
                );
                continue;
            }
        };
        custom.push((KeybindingKeystroke::from_keystroke(keystroke), bytes));
    }
    custom
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
