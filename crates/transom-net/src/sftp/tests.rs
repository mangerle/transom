//! SFTP 模块单元测试。

use super::client::{MAX_LIST_ENTRIES, list_limit_reached};

/// 上限边界：恰好等于上限即拒绝追加，留一条余量在限内放行。
#[test]
fn list_limit_boundary() {
    assert!(!list_limit_reached(MAX_LIST_ENTRIES - 1));
    assert!(list_limit_reached(MAX_LIST_ENTRIES));
    assert!(list_limit_reached(MAX_LIST_ENTRIES + 1));
}
