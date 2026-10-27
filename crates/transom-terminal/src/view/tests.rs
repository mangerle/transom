use crate::view::build_custom_keys;
use crate::view_ime::ime_column_to_utf16_index;
use gpui_kit::Keystroke;
use transom_core::TerminalKeyBinding;

/// 自定义映射按 gpui 键位语法匹配，修饰键不同不得误命中。
#[test]
fn custom_keys_match_exact_modifiers_and_decode_bytes() {
    let bindings = vec![TerminalKeyBinding {
        keys: "ctrl-shift-left".into(),
        sequence: "\\e[1;6D".into(),
    }];
    let custom = build_custom_keys(&bindings);
    assert_eq!(custom.len(), 1);
    assert_eq!(custom[0].1, b"\x1b[1;6D");
    let exact = Keystroke::parse("ctrl-shift-left").unwrap();
    assert!(exact.should_match(&custom[0].0));
    let wrong = Keystroke::parse("ctrl-left").unwrap();
    assert!(!wrong.should_match(&custom[0].0));
}

/// 坏映射在构造阶段被跳过，不能留下半解析状态。
#[test]
fn invalid_custom_keys_are_skipped() {
    let bindings = vec![
        TerminalKeyBinding {
            keys: "not-a-real-key-".into(),
            sequence: "\\e".into(),
        },
        TerminalKeyBinding {
            keys: "ctrl-a".into(),
            sequence: "\\q".into(),
        },
    ];
    assert!(build_custom_keys(&bindings).is_empty());
}

/// IME 定位折算：ASCII 单格、非 ASCII 双格近似，越界收敛到端点。
#[test]
fn ime_column_maps_cells_to_utf16_index() {
    // "ab😀" 的格占用为 1+1+2；UTF-16 长度为 4（表情符号占两个单元）。
    assert_eq!(ime_column_to_utf16_index("ab😀", 0), 0);
    assert_eq!(ime_column_to_utf16_index("ab😀", 1), 1);
    // 第 2 格已进入表情符号的区间，返回其起点下标 2
    assert_eq!(ime_column_to_utf16_index("ab😀", 2), 2);
    // 越过组合文本末尾收敛到全长
    assert_eq!(ime_column_to_utf16_index("ab😀", 9), 4);
    assert_eq!(ime_column_to_utf16_index("", 3), 0);
}
