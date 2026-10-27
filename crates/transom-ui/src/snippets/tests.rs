//! 快捷命令预览与截断单元测试。

use super::{SNIPPET_PREVIEW_MAX_CHARS, snippet_preview};

#[test]
fn preview_keeps_first_line_and_marks_truncation() {
    assert_eq!(snippet_preview("df -h"), "df -h");
    assert_eq!(snippet_preview("df -h\nfree -h"), "df -h …");
    let long = "x".repeat(SNIPPET_PREVIEW_MAX_CHARS + 5);
    let preview = snippet_preview(&long);
    assert!(preview.ends_with(" …"));
    assert_eq!(preview.chars().count(), SNIPPET_PREVIEW_MAX_CHARS + 2);
}
