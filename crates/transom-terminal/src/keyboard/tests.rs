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
