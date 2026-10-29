//! Kitty 扩展键盘协议（CSI u）编码器。
//!
//! 设计原理（Why）：
//! - `alacritty_terminal` 已经解析 `CSI ? u` 查询、`CSI = flags u` 设置与
//!   `CSI > / CSI <` 压栈，并把五个渐进增强标志映射到 [`TermMode`]。本模块只负责
//!   "按键 → 字节"的另一半，不维护第二份标志状态，避免与内核解析结果分叉；
//! - 传统编码把 Ctrl+I 与 Tab、Ctrl+M 与 Enter、Ctrl+[ 与 ESC 编成同一字节，
//!   应用无法区分。Kitty 协议用 `CSI 码;修饰键[:事件类型];文本 u` 精确表达，
//!   只有应用显式协商后才切换编码，未协商时原样走传统序列，老程序行为不变；
//! - `Enter` / `Tab` / `Backspace` 在只开启 disambiguate 时仍保留传统字节：这是
//!   协议刻意留下的逃生门，程序崩溃没清模式时用户仍能敲 `reset`。带修饰键或
//!   开启 "all keys as escape codes" 后才改走 CSI u。
//!
//! 代价与局限：
//! - GPUI 的 [`Keystroke`] 不暴露键盘布局的物理键位，无法填写 base-layout 备用键；
//!   备用键只上报 shifted 码（协议允许省略 base-layout）；
//! - 左右修饰键无法区分，修饰键自身统一按左侧码值上报；
//! - 上游 `alacritty_terminal` 的 `CSI ? u` 查询回复读取的是压栈栈顶而非当前
//!   生效标志：标准握手（push → query）结果正确，但"先 set 再 query"可能拿到
//!   上一次栈顶值。该行为在上游修复前不在本项目重复维护第二份标志状态。

use alacritty_terminal::term::TermMode;
use gpui_kit::Keystroke;

/// 一次按键在事件流中的类型。
///
/// 传统编码只有按下/重复两种语义；释放事件仅在应用协商 `REPORT_EVENT_TYPES`
/// 后才发送，因此调用方必须把平台事件如实传进来，不能一律当按下处理。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyEventKind {
    Press,
    Repeat,
    Release,
}

/// 编码一次按键；返回 `None` 表示"保持传统编码/文本输入路径"。
pub(crate) fn encode_kitty(
    keystroke: &Keystroke,
    mode: TermMode,
    kind: KeyEventKind,
) -> Option<Vec<u8>> {
    let modifiers = keystroke.modifiers;
    let mods = modifier_bits(modifiers);
    let all_keys = mode.contains(TermMode::REPORT_ALL_KEYS_AS_ESC);
    let report_events = mode.contains(TermMode::REPORT_EVENT_TYPES);
    if kind == KeyEventKind::Release && !report_events {
        return None;
    }
    let event = event_type(kind, report_events);

    if let Some(code) = special_key_code(&keystroke.key) {
        // 协议逃生门：未开启 all-keys 时，无修饰的 Enter/Tab/Backspace/Esc 走传统字节。
        if mods == 1 && !all_keys {
            return None;
        }
        // Enter/Tab/Backspace 的释放事件同样受逃生门限制（协议明文规定）。
        if kind == KeyEventKind::Release && !all_keys && code != 27 {
            return None;
        }
        return Some(csi_u(code, mods, event, &[]));
    }

    if let Some(functional) = functional_key(&keystroke.key) {
        // 无修饰且只关心按下时，箭头/功能键沿用传统序列（含 APP_CURSOR 分支）。
        if mods == 1 && kind == KeyEventKind::Press {
            return None;
        }
        return Some(functional.encode(mods, event));
    }

    if let Some(code) = modifier_key_code(&keystroke.key) {
        if !all_keys {
            return None;
        }
        return Some(csi_u(code, mods, event, &[]));
    }

    let ch = if keystroke.key == "space" {
        ' '
    } else {
        single_char(&keystroke.key)?
    };
    let has_command_modifier =
        modifiers.control || modifiers.alt || modifiers.platform || modifiers.function;
    if !all_keys && !has_command_modifier {
        // 普通可打印键继续交给 IME/文本输入路径，保持输入法组合与死键行为。
        return None;
    }
    if kind == KeyEventKind::Release && !report_events {
        return None;
    }

    let code = ch as u32;
    let shifted = mode
        .contains(TermMode::REPORT_ALTERNATE_KEYS)
        .then(|| shifted_code(keystroke, ch))
        .flatten();
    let text = associated_text(keystroke, mode, modifiers, kind);
    Some(csi_u_rich(code, shifted, mods, event, &text))
}

/// 修饰键位图：协议规定 `1 + bitfield`。
fn modifier_bits(modifiers: gpui_kit::Modifiers) -> u8 {
    // 协议值是 `1 + 位图`：shift=1、alt=2、ctrl=4、super=8。位图从 0 开始，
    // 最后统一加 1，避免把 shift 的 0b1 与基准 1 混成同一个值。
    let mut bits = 0u8;
    if modifiers.shift {
        bits |= 0b1;
    }
    if modifiers.alt {
        bits |= 0b10;
    }
    if modifiers.control {
        bits |= 0b100;
    }
    if modifiers.platform {
        bits |= 0b1000;
    }
    bits.saturating_add(1)
}

/// 事件类型子字段；按下是默认值，省略以贴近协议常见形态。
fn event_type(kind: KeyEventKind, report_events: bool) -> Option<u8> {
    if !report_events {
        return None;
    }
    match kind {
        KeyEventKind::Press => None,
        KeyEventKind::Repeat => Some(2),
        KeyEventKind::Release => Some(3),
    }
}

/// 有独立码值的文本类控制键。
fn special_key_code(key: &str) -> Option<u32> {
    match key {
        "escape" => Some(27),
        "enter" => Some(13),
        "tab" => Some(9),
        "backspace" => Some(127),
        _ => None,
    }
}

/// 具备传统功能键序列的按键。
enum Functional {
    /// `CSI number;mods~`。
    Tilde(u32),
    /// `CSI 1;mods{letter}`。
    Cursor(u8),
    /// 没有传统序列、只能走 CSI u 的私有区码值。
    Private(u32),
}

impl Functional {
    fn encode(&self, mods: u8, event: Option<u8>) -> Vec<u8> {
        match self {
            Self::Tilde(number) => csi_functional(*number, mods, event, b'~'),
            Self::Cursor(letter) => csi_functional(1, mods, event, *letter),
            Self::Private(code) => csi_u(*code, mods, event, &[]),
        }
    }
}

fn functional_key(key: &str) -> Option<Functional> {
    Some(match key {
        "up" => Functional::Cursor(b'A'),
        "down" => Functional::Cursor(b'B'),
        "right" => Functional::Cursor(b'C'),
        "left" => Functional::Cursor(b'D'),
        "home" => Functional::Cursor(b'H'),
        "end" => Functional::Cursor(b'F'),
        "insert" => Functional::Tilde(2),
        "delete" => Functional::Tilde(3),
        "pageup" => Functional::Tilde(5),
        "pagedown" => Functional::Tilde(6),
        "f1" => Functional::Cursor(b'P'),
        "f2" => Functional::Cursor(b'Q'),
        "f3" => Functional::Tilde(13),
        "f4" => Functional::Cursor(b'S'),
        "f5" => Functional::Tilde(15),
        "f6" => Functional::Tilde(17),
        "f7" => Functional::Tilde(18),
        "f8" => Functional::Tilde(19),
        "f9" => Functional::Tilde(20),
        "f10" => Functional::Tilde(21),
        "f11" => Functional::Tilde(23),
        "f12" => Functional::Tilde(24),
        "capslock" => Functional::Private(57358),
        "scrolllock" => Functional::Private(57359),
        "numlock" => Functional::Private(57360),
        "printscreen" => Functional::Private(57361),
        "pause" => Functional::Private(57362),
        "menu" => Functional::Private(57363),
        _ => return None,
    })
}

/// 修饰键自身的码值（左侧）。只在应用要求"全部按键都上报"时发送。
fn modifier_key_code(key: &str) -> Option<u32> {
    match key {
        "shift" => Some(57441),
        "control" | "ctrl" => Some(57442),
        "alt" | "option" => Some(57443),
        "super" | "platform" | "cmd" | "command" | "win" => Some(57444),
        _ => None,
    }
}

fn single_char(text: &str) -> Option<char> {
    let mut chars = text.chars();
    match (chars.next(), chars.next()) {
        (Some(ch), None) => Some(ch),
        _ => None,
    }
}

/// 应用协商备用键上报时，计算 shift 后的字符码。
///
/// GPUI 的 `key` 恒定是未上档字符，`key_char` 才是实际输入字符；两者不同才说明
/// 该键在当前布局下有独立的上档形态。
fn shifted_code(keystroke: &Keystroke, base: char) -> Option<u32> {
    if !keystroke.modifiers.shift {
        return None;
    }
    let shifted = keystroke
        .key_char
        .as_deref()
        .and_then(single_char)
        .filter(|ch| *ch != base)?;
    Some(shifted as u32)
}

/// 应用协商关联文本上报时，把本次按键实际产生的文本折算成码点序列。
///
/// 只有不带 Ctrl/Alt/Super 的按键才会产生可打印文本；释放事件不再重复携带文本。
fn associated_text(
    keystroke: &Keystroke,
    mode: TermMode,
    modifiers: gpui_kit::Modifiers,
    kind: KeyEventKind,
) -> Vec<u32> {
    if !mode.contains(TermMode::REPORT_ASSOCIATED_TEXT)
        || kind == KeyEventKind::Release
        || modifiers.control
        || modifiers.alt
        || modifiers.platform
        || modifiers.function
    {
        return Vec::new();
    }
    let Some(text) = keystroke.key_char.as_deref() else {
        return Vec::new();
    };
    text.chars()
        .filter(|ch| !ch.is_control())
        .map(|ch| ch as u32)
        .collect()
}

/// 组出 `CSI 码[:备用键];修饰键[:事件];文本 u`。
fn csi_u_rich(
    code: u32,
    shifted: Option<u32>,
    mods: u8,
    event: Option<u8>,
    text: &[u32],
) -> Vec<u8> {
    let mut out = String::with_capacity(24);
    out.push_str("\x1b[");
    out.push_str(&code.to_string());
    if let Some(shifted) = shifted {
        out.push(':');
        out.push_str(&shifted.to_string());
    }
    if mods != 1 || event.is_some() || !text.is_empty() {
        out.push(';');
        out.push_str(&mods.to_string());
        if let Some(event) = event {
            out.push(':');
            out.push_str(&event.to_string());
        }
    }
    if !text.is_empty() {
        out.push(';');
        for (index, codepoint) in text.iter().enumerate() {
            if index > 0 {
                out.push(':');
            }
            out.push_str(&codepoint.to_string());
        }
    }
    out.push('u');
    out.into_bytes()
}

fn csi_u(code: u32, mods: u8, event: Option<u8>, text: &[u32]) -> Vec<u8> {
    csi_u_rich(code, None, mods, event, text)
}

/// 组出传统功能键形态 `CSI number;mods[:事件]终字节`。
fn csi_functional(number: u32, mods: u8, event: Option<u8>, final_byte: u8) -> Vec<u8> {
    let mut out = String::with_capacity(12);
    out.push_str("\x1b[");
    out.push_str(&number.to_string());
    if mods != 1 || event.is_some() {
        out.push(';');
        out.push_str(&mods.to_string());
        if let Some(event) = event {
            out.push(':');
            out.push_str(&event.to_string());
        }
    }
    out.push(final_byte as char);
    out.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::{KeyEventKind, encode_kitty};
    use alacritty_terminal::term::TermMode;
    use gpui_kit::{Keystroke, Modifiers};

    fn keystroke(key: &str, key_char: Option<&str>, modifiers: Modifiers) -> Keystroke {
        Keystroke {
            modifiers,
            key: key.to_string(),
            key_char: key_char.map(str::to_string),
        }
    }

    fn disambiguate() -> TermMode {
        TermMode::DISAMBIGUATE_ESC_CODES
    }

    fn all_keys() -> TermMode {
        TermMode::DISAMBIGUATE_ESC_CODES | TermMode::REPORT_ALL_KEYS_AS_ESC
    }

    fn ctrl() -> Modifiers {
        Modifiers {
            control: true,
            ..Modifiers::default()
        }
    }

    fn shift() -> Modifiers {
        Modifiers {
            shift: true,
            ..Modifiers::default()
        }
    }

    fn alt() -> Modifiers {
        Modifiers {
            alt: true,
            ..Modifiers::default()
        }
    }

    fn encode(key: &str, key_char: Option<&str>, modifiers: Modifiers, mode: TermMode) -> String {
        let bytes = encode_kitty(
            &keystroke(key, key_char, modifiers),
            mode,
            KeyEventKind::Press,
        )
        .expect("该按键应产生 CSI u 编码");
        String::from_utf8(bytes).expect("编码应为 UTF-8")
    }

    /// 只开 disambiguate 时，Ctrl+Enter 与 Enter 必须彻底分开。
    #[test]
    fn disambiguates_modified_enter_and_tab() {
        assert_eq!(encode("enter", None, ctrl(), disambiguate()), "\x1b[13;5u");
        assert_eq!(encode("enter", None, shift(), disambiguate()), "\x1b[13;2u");
        assert_eq!(encode("enter", None, alt(), disambiguate()), "\x1b[13;3u");
        assert_eq!(encode("tab", None, ctrl(), disambiguate()), "\x1b[9;5u");
        assert_eq!(
            encode(
                "tab",
                None,
                Modifiers {
                    control: true,
                    shift: true,
                    ..Modifiers::default()
                },
                disambiguate()
            ),
            "\x1b[9;6u"
        );
    }

    /// Ctrl+I 在协议下是字母 i，不再是 Tab 的 0x09。
    #[test]
    fn ctrl_i_is_letter_not_tab() {
        assert_eq!(
            encode("i", Some("i"), ctrl(), disambiguate()),
            "\x1b[105;5u"
        );
        assert_eq!(
            encode(
                "i",
                Some("I"),
                Modifiers {
                    control: true,
                    shift: true,
                    ..Modifiers::default()
                },
                disambiguate()
            ),
            "\x1b[105;6u",
            "码值必须是未上档的 105，而不是 73"
        );
    }

    /// 无修饰的 Enter/Tab/Backspace 保留传统字节，保证 `reset` 逃生门可用。
    #[test]
    fn unmodified_enter_tab_backspace_stay_legacy() {
        assert!(
            encode_kitty(
                &keystroke("enter", None, Modifiers::default()),
                disambiguate(),
                KeyEventKind::Press
            )
            .is_none()
        );
        assert!(
            encode_kitty(
                &keystroke("tab", None, Modifiers::default()),
                disambiguate(),
                KeyEventKind::Press
            )
            .is_none()
        );
        assert!(
            encode_kitty(
                &keystroke("a", Some("a"), Modifiers::default()),
                disambiguate(),
                KeyEventKind::Press
            )
            .is_none()
        );
    }

    /// 箭头键带修饰时使用传统 CSI 1;mods 字母形态，而不是私有区码值。
    #[test]
    fn modified_arrows_use_canonical_functional_form() {
        assert_eq!(encode("up", None, shift(), disambiguate()), "\x1b[1;2A");
        assert_eq!(encode("delete", None, ctrl(), disambiguate()), "\x1b[3;5~");
        assert_eq!(encode("f5", None, ctrl(), disambiguate()), "\x1b[15;5~");
    }

    /// 开启全部按键上报后，普通字符也必须变成 CSI u，且关联文本作为第三个字段。
    #[test]
    fn all_keys_reports_plain_text_and_associated_text() {
        assert_eq!(
            encode("a", Some("a"), Modifiers::default(), all_keys()),
            "\x1b[97u"
        );
        let with_text = TermMode::DISAMBIGUATE_ESC_CODES
            | TermMode::REPORT_ALL_KEYS_AS_ESC
            | TermMode::REPORT_ASSOCIATED_TEXT;
        assert_eq!(
            encode("a", Some("A"), shift(), with_text),
            "\x1b[97;2;65u",
            "shift+a 的文本 A 必须作为码点 65 上报"
        );
    }

    /// 备用键：shift+a 同时给出上档码 65，供应用做快捷键匹配。
    #[test]
    fn alternate_keys_report_shifted_codepoint() {
        let mode = TermMode::DISAMBIGUATE_ESC_CODES
            | TermMode::REPORT_ALL_KEYS_AS_ESC
            | TermMode::REPORT_ALTERNATE_KEYS;
        assert_eq!(encode("a", Some("A"), shift(), mode), "\x1b[97:65;2u");
    }

    /// 事件类型：重复与释放以修饰键子字段上报；传统模式下释放事件直接丢弃。
    #[test]
    fn reports_repeat_and_release_events() {
        let mode = TermMode::DISAMBIGUATE_ESC_CODES | TermMode::REPORT_EVENT_TYPES;
        let release = encode_kitty(
            &keystroke("escape", None, ctrl()),
            mode,
            KeyEventKind::Release,
        )
        .expect("开启事件上报后 Escape 必须有释放事件");
        assert_eq!(String::from_utf8(release).unwrap(), "\x1b[27;5:3u");
        let repeat = encode_kitty(
            &keystroke("up", None, Modifiers::default()),
            mode,
            KeyEventKind::Repeat,
        )
        .expect("功能键重复事件应上报");
        assert_eq!(String::from_utf8(repeat).unwrap(), "\x1b[1;1:2A");
        assert!(
            encode_kitty(
                &keystroke("enter", None, Modifiers::default()),
                mode,
                KeyEventKind::Release
            )
            .is_none(),
            "未开启 all-keys 时 Enter 的释放事件必须省略"
        );
    }

    /// Ctrl+Space 不再发 NUL，而是与 Ctrl+其他字符一致的 CSI u。
    #[test]
    fn ctrl_space_uses_csi_u() {
        assert_eq!(
            encode("space", Some(" "), ctrl(), disambiguate()),
            "\x1b[32;5u"
        );
    }
}
