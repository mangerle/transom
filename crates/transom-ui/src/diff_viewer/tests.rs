//! 对比面板的纯函数与本地文件读写测试。

use super::local_io::{read_local_text, write_local_text};
use super::{join_remote, sanitize_label};

#[test]
fn remote_join_handles_root_and_nested_directories() {
    assert_eq!(join_remote("/", "a.txt"), "/a.txt");
    assert_eq!(
        join_remote("/etc/nginx", "nginx.conf"),
        "/etc/nginx/nginx.conf"
    );
    assert_eq!(
        join_remote("/etc/nginx/", "nginx.conf"),
        "/etc/nginx/nginx.conf"
    );
}

#[test]
fn diff_header_labels_strip_line_breaks() {
    assert_eq!(sanitize_label("a/x\ty\nz"), "a/x y z");
}

#[test]
fn local_text_round_trips_through_atomic_write() {
    let dir = tempfile::tempdir().expect("临时目录");
    let path = dir.path().join("app.conf");
    std::fs::write(&path, "旧内容\n").expect("预置文件");

    write_local_text(&path, "旧内容\n", "新内容\n第二行\n").expect("写入应成功");
    assert_eq!(
        read_local_text(&path).expect("读取应成功"),
        "新内容\n第二行\n"
    );
    let leftovers: Vec<_> = std::fs::read_dir(dir.path())
        .expect("枚举目录")
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().contains("transom-tmp"))
        .collect();
    assert!(leftovers.is_empty(), "临时文件必须被清理");
}

#[test]
fn stale_baseline_blocks_local_overwrite() {
    let dir = tempfile::tempdir().expect("临时目录");
    let path = dir.path().join("app.conf");
    std::fs::write(&path, "别人刚改过\n").expect("预置文件");
    let error = write_local_text(&path, "对比时的旧内容\n", "合并结果\n")
        .expect_err("基线不一致必须拒绝覆盖");
    assert_eq!(
        error,
        rust_i18n::t!("diff.error_modified", path = path.display()).to_string(),
        "错误应说明冲突"
    );
    assert_eq!(
        read_local_text(&path).expect("读取"),
        "别人刚改过\n",
        "拒绝覆盖时原文件必须保持不变"
    );
}

#[test]
fn non_utf8_local_file_is_rejected() {
    let dir = tempfile::tempdir().expect("临时目录");
    let path = dir.path().join("gbk.txt");
    std::fs::write(&path, [0xB0, 0xA1, 0xB0, 0xA1]).expect("写入非法 UTF-8");
    let error = read_local_text(&path).expect_err("非 UTF-8 文件必须拒绝");
    assert!(error.contains("UTF-8"), "错误应说明编码问题: {error}");
}
