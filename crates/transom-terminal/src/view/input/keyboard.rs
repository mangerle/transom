//! keyboard 键鼠/输入处理。

use super::custom_keys::is_control_or_function_key;
use super::{SendBacktab, SendTab, TerminalCopy, TerminalPaste, TerminalView};
use crate::input::{
    KeyEventKind, keystroke_event_to_bytes, tab_bytes,
};
use alacritty_terminal::term::TermMode;
use gpui_kit::*;

impl TerminalView {
    pub(crate) fn on_key_down(
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

        // 复制模式是模态的：所有按键都在模式内消费，绝不能写进远端。
        // 应用级快捷键已在上面的 key_filter 处截走，退出组合键仍可用。
        if self.copy_mode_active() {
            self.handle_copy_mode_key(event, cx);
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

        // 复制/粘贴的键位不在这里硬编码：它们登记在应用的可改键表里（默认 `$app-c`
        // /`$app-v`，非 macOS 展开为 Ctrl+Shift+…），命中保留键后由本视图的
        // `on_action_terminal_copy` / `on_action_terminal_paste` 处理，用户可改键。
        // 纯 Ctrl+V 默认被拦下，既不发给 Shell 也不改写成粘贴：`^V` 一旦落到
        // readline 就布置了一次性引用插入，紧接着的括号粘贴会把标记的开头 ESC
        // 变成 `^[[200~` 字面残字并污染那条命令行。拦下即从源头消除误伤。
        // 需要 readline 引用插入或 vim 块选择时关掉本开关回到 Unix 惯例，或保留
        // 开关、用自定义按键把别的组合（如 ctrl-q）绑成 \x16；自定义映射在前面已优先。
        if self.interaction.suppress_ctrl_v
            && modifiers.control
            && !modifiers.shift
            && !modifiers.platform
            && key == "v"
        {
            cx.stop_propagation();
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

    pub(crate) fn on_key_up(&mut self, event: &KeyUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        // 复制模式吃掉了 key down，release 也不能漏给远端：只上报按下不收
        // 松开的程序会看到一组没有对应按下的释放事件。
        if self.copy_mode_active() {
            return;
        }
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

    pub(crate) fn on_action_terminal_copy(
        &mut self,
        _: &TerminalCopy,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.copy_selection(cx);
        if self.copy_mode_active() {
            self.exit_copy_mode(cx);
        }
    }

    pub(crate) fn on_action_terminal_paste(
        &mut self,
        _: &TerminalPaste,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.copy_mode_active() {
            self.exit_copy_mode(cx);
        }
        self.paste_from_clipboard(cx);
    }

    pub(crate) fn on_action_send_tab(
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

    pub(crate) fn on_action_send_backtab(
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
}
