//! 审计日志查看器单元测试。

use super::types::{
    find_session_audit_logs, format_file_size, read_log_content, strip_ansi_escapes,
};

#[test]
fn test_strip_ansi_escapes() {
    let input = "\x1b[31mRed Text\x1b[0m and \x1b[1;32mGreen\x1b[0m\r\nNext Line";
    let cleaned = strip_ansi_escapes(input);
    assert_eq!(cleaned, "Red Text and Green\nNext Line");
}

#[test]
fn test_format_file_size() {
    assert_eq!(format_file_size(500), "500 B");
    assert_eq!(format_file_size(2048), "2.0 KiB");
    assert_eq!(format_file_size(5 * 1024 * 1024), "5.0 MiB");
}

/// 超过上限的日志只读取上限字节并标记截断。
#[test]
fn read_log_content_truncates_at_limit() {
    let dir = tempfile::tempdir().expect("临时目录");
    let path = dir.path().join("audit.log");
    std::fs::write(&path, "abcdef").expect("写入样本");
    let (content, truncated) = read_log_content(&path, 3);
    assert_eq!(content, "abc");
    assert!(truncated);
}

/// 文件不存在时返回可展示的错误文本，而不是空内容或崩溃。
#[test]
fn read_log_content_reports_missing_file() {
    let path = std::path::Path::new("/definitely/not/a/transom-audit.log");
    let (content, truncated) = read_log_content(path, 16);
    assert!(!truncated);
    assert!(!content.is_empty());
}

/// 默认只看当前会话的日志前缀；show_all 时列出全部 .log 并忽略其它扩展名。
#[test]
fn finds_session_logs_by_prefix_and_extension() {
    let dir = tempfile::tempdir().expect("临时目录");
    for name in [
        "alpha-host-1.log",
        "alpha-host-2.log",
        "beta-other-1.log",
        "notes.txt",
    ] {
        std::fs::write(dir.path().join(name), "x").expect("写入样本");
    }
    let filtered = find_session_audit_logs(dir.path(), Some("alpha"), Some("host"), false);
    assert_eq!(filtered.len(), 2);
    // 文件名倒序：最新的 alpha-host-2 排在前面。
    assert_eq!(filtered[0].file_name, "alpha-host-2.log");
    let all = find_session_audit_logs(dir.path(), None, None, true);
    assert_eq!(all.len(), 3);
}
