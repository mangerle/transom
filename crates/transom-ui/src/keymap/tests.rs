//! 快捷键解析、冲突检测与终端按键绑定单元测试。

use super::{
    CaptureIssue, SPECS, ShortcutSettings, conflict_with, effective_key_strings, effective_keys,
    expand_app_placeholder, format_captured, parse_terminal_key_bindings, spec_by_id,
    suppression_bindings,
};
use gpui_kit::Keystroke;

fn custom(id: &str, keys: &[&str]) -> ShortcutSettings {
    let mut settings = ShortcutSettings::default();
    settings.bindings.insert(
        id.to_string(),
        keys.iter().map(|key| key.to_string()).collect(),
    );
    settings
}

#[test]
fn defaults_expand_platform_prefix_and_include_alternatives() {
    let settings = ShortcutSettings::default();
    let new_session = spec_by_id("new_session").unwrap();
    let keys = effective_keys(new_session, &settings);
    assert_eq!(keys.len(), 1);
    assert!(keys[0].contains("n"));
    let immersive = spec_by_id("immersive").unwrap();
    assert_eq!(
        effective_keys(immersive, &settings),
        vec!["f11", "alt-enter"]
    );
}

#[test]
fn invalid_custom_keys_fall_back_to_defaults() {
    let settings = custom("new_session", &["not-a-real-key!!"]);
    let spec = spec_by_id("new_session").unwrap();
    assert_eq!(
        effective_keys(spec, &settings),
        vec![expand_app_placeholder("$app-n")]
    );
}

#[test]
fn duplicate_bindings_are_detected() {
    let settings = custom("new_session", &["ctrl-shift-q"]);
    let spec = spec_by_id("new_session").unwrap();
    let keys = effective_keys(spec, &settings);
    assert_eq!(keys, vec!["ctrl-shift-q"]);
    assert_eq!(
        conflict_with("quick_open", "ctrl-shift-q", &settings).map(|spec| spec.id),
        Some("new_session")
    );
}

#[test]
fn suppression_covers_every_effective_key() {
    let settings = ShortcutSettings::default();
    let keys = effective_key_strings(&settings);
    assert_eq!(keys.len(), SPECS.len() + 1);
    assert_eq!(suppression_bindings(&keys).len(), keys.len());
}

#[test]
fn capture_requires_non_shift_modifier_or_function_key() {
    let plain = Keystroke::parse("n").unwrap();
    assert_eq!(format_captured(&plain), Err(CaptureIssue::NeedsModifier));
    let shift = Keystroke::parse("shift-n").unwrap();
    assert_eq!(format_captured(&shift), Err(CaptureIssue::NeedsModifier));
    let ctrl = Keystroke::parse("ctrl-shift-n").unwrap();
    assert_eq!(format_captured(&ctrl).as_deref(), Ok("ctrl-shift-n"));
    let f11 = Keystroke::parse("f11").unwrap();
    assert_eq!(format_captured(&f11).as_deref(), Ok("f11"));
}

#[test]
fn accepts_modifier_and_function_keys() {
    let bindings = parse_terminal_key_bindings(
        "ctrl-shift-left = \\e[1;6D\nf1 = \\eOP\nalt-backspace = \\x7f",
    )
    .unwrap();
    assert_eq!(bindings.len(), 3);
}

#[test]
fn rejects_plain_typing_override_and_duplicates() {
    let error = parse_terminal_key_bindings("a = \\e").unwrap_err();
    assert!(
        error.contains(rust_i18n::t!("keymap.missing_modifier", keys = "a").as_ref()),
        "错误: {error}"
    );
    let error = parse_terminal_key_bindings("space = \\e").unwrap_err();
    assert!(
        error.contains(rust_i18n::t!("keymap.missing_modifier", keys = "space").as_ref()),
        "错误: {error}"
    );
    let error = parse_terminal_key_bindings("ctrl-a = \\e\nctrl-a = \\x01").unwrap_err();
    assert!(
        error.contains(
            rust_i18n::t!("keymap.duplicate", keys = "ctrl-a", previous = "ctrl-a").as_ref()
        ),
        "错误: {error}"
    );
}
