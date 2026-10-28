use super::path::*;
use super::types::*;

#[test]
fn test_octal_and_rwx_permissions() {
    assert_eq!(format_octal_permissions(0o100755), "755");
    assert_eq!(format_octal_permissions(0o100644), "644");
    assert_eq!(format_rwx_permissions(0o755), "rwxr-xr-x");
    assert_eq!(format_rwx_permissions(0o644), "rw-r--r--");
    assert_eq!(parse_octal_permissions("755"), Some(0o755));
    assert_eq!(parse_octal_permissions("0644"), Some(0o644));
    assert_eq!(parse_octal_permissions("999"), None);
    assert_eq!(parse_octal_permissions("1777"), None);
}

#[test]
fn test_remote_text_meta_conflict_detection() {
    let opened = RemoteTextMeta {
        size: 100,
        mtime: Some(1000),
        bom: false,
        crlf: false,
        transcoded: false,
    };
    // 远端与打开时完全一致：不冲突。
    assert!(!opened.conflicts_with(&RemoteTextMeta {
        size: 100,
        mtime: Some(1000),
        bom: false,
        crlf: false,
        transcoded: false,
    }));
    // 大小变化（含保存后内容变短/变长）：冲突。
    assert!(opened.conflicts_with(&RemoteTextMeta {
        size: 90,
        mtime: Some(1000),
        bom: false,
        crlf: false,
        transcoded: false,
    }));
    // 大小相同但 mtime 变化（同尺寸被改写）：冲突。
    assert!(opened.conflicts_with(&RemoteTextMeta {
        size: 100,
        mtime: Some(2000),
        bom: false,
        crlf: false,
        transcoded: false,
    }));
}

#[test]
fn test_remote_text_meta_treats_missing_mtime_as_wildcard() {
    // 打开时服务器未回传 mtime：只按大小比对，当前 mtime 出现不视为冲突，
    // 避免偶发缺字段把合法保存误判成冲突。
    let opened = RemoteTextMeta {
        size: 100,
        mtime: None,
        bom: false,
        crlf: false,
        transcoded: false,
    };
    assert!(!opened.conflicts_with(&RemoteTextMeta {
        size: 100,
        mtime: Some(2000),
        bom: false,
        crlf: false,
        transcoded: false,
    }));
    assert!(opened.conflicts_with(&RemoteTextMeta {
        size: 120,
        mtime: Some(2000),
        bom: false,
        crlf: false,
        transcoded: false,
    }));
    // 两侧 mtime 均缺失：退化为纯大小比对。
    assert!(!opened.conflicts_with(&RemoteTextMeta {
        size: 100,
        mtime: None,
        bom: false,
        crlf: false,
        transcoded: false,
    }));
}

#[test]
fn safe_file_name_rejects_path_traversal_and_separators() {
    for name in ["", ".", "..", "/etc/passwd", "../escape", "a/b", "x\0y"] {
        assert!(!is_safe_file_name(name), "应拒绝非法文件名: {name:?}");
    }
    for name in ["app.log", "目录", "a b", ".hidden", "a-b_c"] {
        assert!(is_safe_file_name(name), "应接受普通文件名: {name:?}");
    }
    // 反斜杠是否是分隔符取决于宿主平台：Unix 允许它作为普通字符，
    // Windows 必须拒绝，否则 "..\\escape" 会逃出下载目录。
    #[cfg(unix)]
    assert!(is_safe_file_name("a\\b"));
    #[cfg(windows)]
    {
        assert!(!is_safe_file_name("a\\b"));
        assert!(!is_safe_file_name("..\\escape"));
        assert!(!is_safe_file_name("C:\\escape"));
    }
}

#[test]
fn safe_local_join_stays_inside_download_root() {
    let root = std::path::Path::new("/tmp/transom-download");
    assert_eq!(
        safe_local_join(root, "app.log").expect("普通文件名应可拼接"),
        root.join("app.log")
    );
    assert!(safe_local_join(root, "/etc/passwd").is_err());
    assert!(safe_local_join(root, "../escape").is_err());
    assert!(safe_local_join(root, "a/b").is_err());
}
