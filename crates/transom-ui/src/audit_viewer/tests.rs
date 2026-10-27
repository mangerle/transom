//! 审计日志查看器单元测试。

use super::types::{format_file_size, strip_ansi_escapes};

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
