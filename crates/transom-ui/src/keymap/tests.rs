//! 快捷键解析、冲突检测与终端按键绑定单元测试。

use super::{
    CaptureIssue, NewSession, SPECS, ShortcutSettings, WORKSPACE_ONLY, conflict_with,
    effective_key_strings, effective_keys, expand_app_placeholder, format_captured,
    load_key_binding, parse_terminal_key_bindings, reserved_keystrokes, spec_by_id,
    suppression_bindings,
};
use gpui_kit::{KeybindingKeystroke, Keystroke, OperatingSystem, TestAppContext};

const OS: OperatingSystem = OperatingSystem::Linux;

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
    let keys = effective_keys(new_session, &settings, OS);
    assert_eq!(keys.len(), 1);
    assert!(keys[0].contains("n"));
    let immersive = spec_by_id("immersive").unwrap();
    assert_eq!(
        effective_keys(immersive, &settings, OS),
        vec!["f11", "alt-enter"]
    );
}

#[test]
fn runtime_binding_loader_resolves_secondary_alias() {
    let app = TestAppContext::single();
    app.update(|cx| {
        assert!(
            load_key_binding("secondary-n", NewSession, WORKSPACE_ONLY, cx).is_some(),
            "运行期键位必须能解析 secondary 别名"
        );
        let suppression = suppression_bindings(&[("secondary-n".to_string(), WORKSPACE_ONLY)], cx);
        assert_eq!(suppression.len(), 1);
    });
}

#[test]
fn invalid_custom_keys_fall_back_to_defaults() {
    let settings = custom("new_session", &["not-a-real-key!!"]);
    let spec = spec_by_id("new_session").unwrap();
    assert_eq!(
        effective_keys(spec, &settings, OS),
        vec![expand_app_placeholder("$app-n")]
    );
}

#[test]
fn duplicate_bindings_are_detected() {
    let settings = custom("new_session", &["ctrl-shift-q"]);
    let spec = spec_by_id("new_session").unwrap();
    let keys = effective_keys(spec, &settings, OS);
    assert_eq!(keys, vec!["ctrl-shift-q"]);
    assert_eq!(
        conflict_with("quick_open", "ctrl-shift-q", &settings, OS).map(|spec| spec.id),
        Some("new_session")
    );
}

#[test]
fn suppression_covers_every_effective_key() {
    let settings = ShortcutSettings::default();
    let keys = effective_key_strings(&settings, OS);
    assert_eq!(keys.len(), SPECS.len() + 1);
    assert_eq!(keys.len(), SPECS.len() + 1);
}

/// 面板专属上下文的键位不得进入终端保留名单：一旦收录，终端里的这几个组合键
/// 会被应用吃掉却又不会触发面板动作，vim 翻页、readline 前移这类依赖控制码的
/// 操作会莫名其妙失灵。这里给面板动作各配一把唯一组合，避开 `$app` 占位符在
/// 不同平台上与工作区别名重名的展开。
#[test]
fn panel_context_keys_are_not_reserved_from_terminal() {
    let mut settings = ShortcutSettings::default();
    for (id, key) in [
        ("sftp_filter", "ctrl-alt-f"),
        ("save_editor_file", "ctrl-alt-s"),
        ("go_to_editor_line", "ctrl-alt-g"),
    ] {
        settings
            .bindings
            .insert(id.to_string(), vec![key.to_string()]);
    }
    let reserved = reserved_keystrokes(&settings, OS);
    for key in ["ctrl-alt-f", "ctrl-alt-s", "ctrl-alt-g"] {
        let parsed = KeybindingKeystroke::from_keystroke(Keystroke::parse(key).unwrap());
        assert!(
            !reserved.contains(&parsed),
            "面板专属键位不应被终端保留，键位: {key}"
        );
    }
}

/// 终端上下文的复制/粘贴必须进入保留名单：否则同一次按键既触发应用动作，又被
/// 编码成控制码发给 Shell（`Ctrl+Shift+C` 会编成 0x03 打断前台进程）。
#[test]
fn terminal_copy_paste_keys_are_reserved() {
    let settings = ShortcutSettings::default();
    let reserved = reserved_keystrokes(&settings, OS);
    for id in ["terminal_copy", "terminal_paste"] {
        let spec = spec_by_id(id).unwrap();
        let key = effective_keys(spec, &settings, OS).remove(0);
        let parsed =
            KeybindingKeystroke::from_keystroke(Keystroke::parse_with_os(&key, OS).unwrap());
        assert!(
            reserved.contains(&parsed),
            "终端动作键位应被保留，键位: {key}"
        );
    }
}

#[test]
fn capture_requires_non_shift_modifier_or_function_key() {
    let plain = Keystroke::parse("n").unwrap();
    assert_eq!(
        format_captured(&plain, OS),
        Err(CaptureIssue::NeedsModifier)
    );
    let shift = Keystroke::parse("shift-n").unwrap();
    assert_eq!(
        format_captured(&shift, OS),
        Err(CaptureIssue::NeedsModifier)
    );
    let ctrl = Keystroke::parse("ctrl-shift-n").unwrap();
    assert_eq!(format_captured(&ctrl, OS).as_deref(), Ok("ctrl-shift-n"));
    let f11 = Keystroke::parse("f11").unwrap();
    assert_eq!(format_captured(&f11, OS).as_deref(), Ok("f11"));
}

#[test]
fn accepts_modifier_and_function_keys() {
    let bindings = parse_terminal_key_bindings(
        "ctrl-shift-left = \\e[1;6D\nf1 = \\eOP\nalt-backspace = \\x7f",
        OS,
    )
    .unwrap();
    assert_eq!(bindings.len(), 3);
}

#[test]
fn rejects_plain_typing_override_and_duplicates() {
    let error = parse_terminal_key_bindings("a = \\e", OS).unwrap_err();
    assert!(
        error.contains(rust_i18n::t!("keymap.missing_modifier", keys = "a").as_ref()),
        "错误: {error}"
    );
    let error = parse_terminal_key_bindings("space = \\e", OS).unwrap_err();
    assert!(
        error.contains(rust_i18n::t!("keymap.missing_modifier", keys = "space").as_ref()),
        "错误: {error}"
    );
    let error = parse_terminal_key_bindings("ctrl-a = \\e\nctrl-a = \\x01", OS).unwrap_err();
    assert!(
        error.contains(
            rust_i18n::t!("keymap.duplicate", keys = "ctrl-a", previous = "ctrl-a").as_ref()
        ),
        "错误: {error}"
    );
}
