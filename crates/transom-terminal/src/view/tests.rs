use crate::view::build_custom_keys;
use crate::view_ime::{
    ime_caret_index_for_query, ime_column_to_utf16_index, ime_utf16_index_to_column_cells,
};
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
    // "ab\u{20BB7}" 的格占用为 1+1+2；UTF-16 长度为 4（增补平面汉字占两个码元）。
    assert_eq!(ime_column_to_utf16_index("ab\u{20BB7}", 0), 0);
    assert_eq!(ime_column_to_utf16_index("ab\u{20BB7}", 1), 1);
    // 第 2 格已进入增补汉字的区间，返回其起点下标 2
    assert_eq!(ime_column_to_utf16_index("ab\u{20BB7}", 2), 2);
    // 越过组合文本末尾收敛到全长
    assert_eq!(ime_column_to_utf16_index("ab\u{20BB7}", 9), 4);
    assert_eq!(ime_column_to_utf16_index("", 3), 0);
}

/// IME 定位折算：UTF-16 下标折算成末端列数，与反方向折算保持同一宽度假设。
///
/// 组合文本自绘在光标格起始处，输入法层以选区末端询问组字光标位置，
/// 该折算决定候选窗随组字文本右移的偏移量。
#[test]
fn ime_utf16_index_maps_to_column_cells() {
    // "ab\u{20BB7}" 的格占用为 1+1+2；UTF-16 长度为 4（增补平面汉字占两个码元）。
    assert_eq!(ime_utf16_index_to_column_cells("ab\u{20BB7}", 0), 0);
    assert_eq!(ime_utf16_index_to_column_cells("ab\u{20BB7}", 1), 1);
    assert_eq!(ime_utf16_index_to_column_cells("ab\u{20BB7}", 2), 2);
    assert_eq!(ime_utf16_index_to_column_cells("ab\u{20BB7}", 4), 4);
    // 越过组合文本末尾收敛到全长列数
    assert_eq!(ime_utf16_index_to_column_cells("ab\u{20BB7}", 9), 4);
    assert_eq!(ime_utf16_index_to_column_cells("", 3), 0);
}

/// 候选窗定位下标：落在组合文本内的查询统一取组字光标，范围外保持原样。
///
/// GPUI 会把查询收敛到组字行首，若不改判，单行组字时候选窗会停在组字起点。
#[test]
fn ime_caret_index_prefers_composition_end() {
    let marked = 0..5;
    // 组字内任意查询（含行首探测用的 0..0 与光标 5..5）都折算到组字光标
    assert_eq!(ime_caret_index_for_query(&(0..0), Some(&marked)), 5);
    assert_eq!(ime_caret_index_for_query(&(3..3), Some(&marked)), 5);
    assert_eq!(ime_caret_index_for_query(&(0..5), Some(&marked)), 5);
    // 无组合文本时保持请求下标
    assert_eq!(ime_caret_index_for_query(&(2..2), None), 2);
    // 组合文本范围之外的查询不受影响
    assert_eq!(ime_caret_index_for_query(&(9..9), Some(&marked)), 9);
}

/// 尺寸去抖判定：目标尺寸稳定满窗口才下发，中途换尺寸重新计时。
///
/// 这条语义是终端错位修复的核心：中间尺寸一旦下发，程序就会在旧宽度下继续输出，
/// 本地网格却已按新宽度重排，软换行标记就此写错且无法复原。
#[test]
fn resize_debounce_applies_only_after_size_settles() {
    use crate::view::types::{PendingResize, RESIZE_DEBOUNCE, ResizeDecision, decide_resize};
    use std::time::Instant;
    use transom_core::TerminalSize;

    let current = TerminalSize::new(80, 24);
    let target = TerminalSize::new(60, 24);
    let now = Instant::now();

    assert_eq!(
        decide_resize(current, target, None, now),
        ResizeDecision::Restart,
        "首次出现目标尺寸只重新计时，不下发"
    );

    let pending = PendingResize {
        size: target,
        deadline: now + RESIZE_DEBOUNCE,
    };
    assert_eq!(
        decide_resize(current, target, Some(&pending), now),
        ResizeDecision::Wait,
        "去抖窗口内不下发"
    );
    assert_eq!(
        decide_resize(current, target, Some(&pending), now + RESIZE_DEBOUNCE),
        ResizeDecision::Ready,
        "尺寸稳定满窗口才下发"
    );
    assert_eq!(
        decide_resize(current, TerminalSize::new(40, 24), Some(&pending), now),
        ResizeDecision::Restart,
        "中途换成别的尺寸要重新计时"
    );
    assert_eq!(
        decide_resize(target, target, Some(&pending), now),
        ResizeDecision::Unchanged,
        "尺寸已经一致时不做任何动作"
    );
}
