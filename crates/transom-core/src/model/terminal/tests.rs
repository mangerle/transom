use super::*;
use crate::transport::{TerminalControl, TerminalControlHandle, TerminalSize};

/// 老配置没有 `right_click` 字段：必须按旧开关迁移，不能把用户此前的
/// "右键粘贴"悄悄改成弹菜单。
#[test]
fn legacy_right_click_flag_migrates_to_behavior() {
    let paste: TerminalInteraction = serde_json::from_str(
        r#"{"copy_on_select":true,"right_click_paste":true,"multiline_paste_confirm":false}"#,
    )
    .unwrap();
    assert_eq!(paste.right_click_behavior(), RightClickBehavior::Paste);

    let menu: TerminalInteraction = serde_json::from_str(
        r#"{"copy_on_select":true,"right_click_paste":false,"multiline_paste_confirm":false}"#,
    )
    .unwrap();
    assert_eq!(menu.right_click_behavior(), RightClickBehavior::Menu);
}

/// 新档位与旧布尔字段双向同步：老版本降级读取时语义不反转。
#[test]
fn setting_behavior_updates_legacy_flag() {
    let mut interaction = TerminalInteraction::default();
    interaction.set_right_click_behavior(RightClickBehavior::MenuWithPaste);
    assert!(!interaction.right_click_paste);
    assert_eq!(
        interaction.right_click_behavior(),
        RightClickBehavior::MenuWithPaste
    );
    interaction.set_right_click_behavior(RightClickBehavior::Paste);
    assert!(interaction.right_click_paste);
}

/// 纯 Ctrl+V 默认被终端拦下：透传 `^V` 会让一次误按污染紧接着的括号粘贴
/// （readline 的一次性引用插入把标记的 ESC 变成字面文本）。老配置没有该字段时
/// 按新默认拦下；显式写 `false` 的用户仍回到把 `^V` 发给 Shell 的 Unix 惯例。
#[test]
fn ctrl_v_is_suppressed_by_default_and_respects_explicit_off() {
    assert!(TerminalInteraction::default().suppress_ctrl_v);

    let legacy: TerminalInteraction = serde_json::from_str(
        r#"{"copy_on_select":true,"right_click_paste":true,"multiline_paste_confirm":false}"#,
    )
    .unwrap();
    assert!(legacy.suppress_ctrl_v);

    let mut off = legacy;
    off.suppress_ctrl_v = false;
    let json = serde_json::to_string(&off).unwrap();
    let restored: TerminalInteraction = serde_json::from_str(&json).unwrap();
    assert!(!restored.suppress_ctrl_v);
}

/// 转义序列解码覆盖终端最常用的几种写法。
#[test]
fn key_binding_decodes_escape_sequences() {
    let binding = TerminalKeyBinding {
        keys: "ctrl-shift-left".into(),
        sequence: "\\e[1;6D".into(),
    };
    assert_eq!(binding.decode_sequence().unwrap(), b"\x1b[1;6D");

    let bytes = decode_escape_sequence("\\x1b]0;title\\x07").unwrap();
    assert_eq!(bytes, b"\x1b]0;title\x07");
    assert_eq!(decode_escape_sequence("A\\nB").unwrap(), b"A\nB");
    assert_eq!(decode_escape_sequence("中").unwrap(), "中".as_bytes());
    assert_eq!(
        decode_escape_sequence("\\u{4e2d}").unwrap(),
        "中".as_bytes()
    );
    assert!(decode_escape_sequence("\\").is_err());
    assert!(decode_escape_sequence("\\x1").is_err());
    assert!(decode_escape_sequence("\\q").is_err());
    assert!(decode_escape_sequence("").is_err());
}

/// 多行配置忽略空行与注释，错误带行号；往返序列化保持一致。
#[test]
fn key_bindings_parse_multiline_round_trip() {
    let text = "# tmux/vim 常用序列\nctrl-shift-left = \\e[1;6D\n\nalt-backspace = \\x7f\n";
    let bindings = TerminalKeyBinding::parse_multiline(text).unwrap();
    assert_eq!(bindings.len(), 2);
    assert_eq!(bindings[0].keys, "ctrl-shift-left");
    assert_eq!(
        TerminalKeyBinding::to_multiline(&bindings),
        "ctrl-shift-left = \\e[1;6D\nalt-backspace = \\x7f"
    );

    let error = TerminalKeyBinding::parse_multiline("ctrl-a \\e").unwrap_err();
    assert!(error.contains("第 1 行"), "错误: {error}");
    let error = TerminalKeyBinding::parse_multiline("ctrl-a = ").unwrap_err();
    assert!(error.contains("第 1 行"), "错误: {error}");
}

#[test]
fn control_handle_delivers_messages() {
    let (handle, mut receiver) = TerminalControlHandle::channel(4);
    assert!(handle.resize(TerminalSize::new(120, 40)));
    assert!(handle.close());
    assert_eq!(
        receiver.try_recv().unwrap(),
        TerminalControl::Resize(TerminalSize::new(120, 40))
    );
    assert_eq!(receiver.try_recv().unwrap(), TerminalControl::Close);
}

/// 分隔符属仿真内核配置：老配置没有这个字段时要退回内核默认，而不是加载失败。
#[test]
fn word_separators_default_and_round_trip() {
    let default_separators = TerminalProfile::default().word_separators.clone();
    assert!(default_separators.contains(','));
    assert!(default_separators.contains(' '), "空格默认应断词");
    assert!(default_separators.contains('('));
    let profile = TerminalProfile {
        word_separators: "-/.".into(),
        ..TerminalProfile::default()
    };
    let json = serde_json::to_string(&profile).unwrap();
    let decoded: TerminalProfile = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded.word_separators, "-/.");

    let mut value = serde_json::to_value(&profile).unwrap();
    value.as_object_mut().unwrap().remove("word_separators");
    let legacy: TerminalProfile = serde_json::from_value(value).unwrap();
    assert_eq!(
        legacy.word_separators,
        TerminalProfile::default().word_separators
    );
}

#[test]
fn default_profile_is_stable() {
    let profile = TerminalProfile::default();
    assert_eq!(profile.scrollback, 10_000);
    assert_eq!(profile.cursor_shape, TerminalCursorShape::Block);
    assert_eq!(profile.palette.ansi.len(), 16);
    assert!(TerminalInteraction::default().copy_on_select);
    assert!(TerminalInteraction::default().right_click_paste);
}

/// 双重字体是后加的字段：老的会话与设置文件里没有它，必须按"跟随系统回退"降级，
/// 而不是让整份配置加载失败。
#[test]
fn profile_without_cjk_font_still_loads() {
    let profile = TerminalProfile {
        cjk_font_family: Some("Sarasa Mono SC".into()),
        ..TerminalProfile::default()
    };
    let mut value = serde_json::to_value(&profile).expect("默认配置应可序列化");
    value
        .as_object_mut()
        .expect("配置序列化为对象")
        .remove("cjk_font_family");
    let decoded: TerminalProfile =
        serde_json::from_value(value).expect("缺少 cjk_font_family 的旧配置应能解析");
    assert_eq!(decoded.cjk_font_family, None);
}

/// 双重字体字段必须能原样往返，否则用户在设置里选的中文字体存不住。
#[test]
fn cjk_font_round_trips() {
    let profile = TerminalProfile {
        cjk_font_family: Some("更纱黑体 Mono SC".into()),
        ..TerminalProfile::default()
    };
    let json = serde_json::to_string(&profile).unwrap();
    let decoded: TerminalProfile = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded.cjk_font_family.as_deref(), Some("更纱黑体 Mono SC"));
}

#[test]
fn parses_keywords_across_chinese_separators() {
    assert_eq!(
        TerminalHighlight::parse_keywords("timeout, refused、连接失败;oom  killed"),
        vec!["timeout", "refused", "连接失败", "oom", "killed"]
    );
    assert!(TerminalHighlight::parse_keywords("  , ， 、 ;;  ").is_empty());
    let highlight = TerminalHighlight {
        enabled: true,
        keywords: vec!["oom".into(), "killed".into()],
    };
    assert_eq!(highlight.keywords_joined(), "oom, killed");
}

#[test]
fn sanitize_drops_blank_dedupes_and_caps() {
    let raw = TerminalHighlight {
        enabled: true,
        keywords: vec![
            "".into(),
            "   ".into(),
            "oom".into(),
            "OOM".into(),
            "KILLED".into(),
            "a".repeat(100),
        ],
    };
    let clean = raw.sanitized();
    assert_eq!(clean.keywords, vec!["oom", "KILLED"]);
    let overflow = TerminalHighlight {
        enabled: true,
        keywords: (0..50).map(|i| format!("kw{i}")).collect(),
    };
    assert_eq!(overflow.sanitized().keywords.len(), 32);
}
