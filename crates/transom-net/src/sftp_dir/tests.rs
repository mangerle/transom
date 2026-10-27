//! SFTP 目录传输路径解析与防遍历单元测试。

use super::{join_remote_path, local_path_for, safe_component};
use std::path::Path;

#[test]
fn rejects_path_traversal_components() {
    assert!(local_path_for(Path::new("/tmp/root"), "../escape").is_err());
    assert!(local_path_for(Path::new("/tmp/root"), "a/../b").is_err());
    assert!(local_path_for(Path::new("/tmp/root"), "a\\b").is_err());
    assert!(local_path_for(Path::new("/tmp/root"), "").is_err());
    assert_eq!(
        local_path_for(Path::new("/tmp/root"), "a/b.txt").expect("合法路径"),
        Path::new("/tmp/root/a/b.txt")
    );
}

#[test]
fn component_validation_and_remote_join() {
    assert!(safe_component("normal.txt"));
    assert!(!safe_component(".."));
    assert!(!safe_component("a/b"));
    assert_eq!(join_remote_path("/", "a/b"), "/a/b");
    assert_eq!(join_remote_path("/srv/app/", "a/b"), "/srv/app/a/b");
    assert_eq!(join_remote_path("relative", "a"), "relative/a");
}
