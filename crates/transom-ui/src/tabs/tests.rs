//! tabs 模块单元测试。

use super::duplicate::{duplicate_base, strip_copy_suffix};

#[test]
fn duplicate_base_strips_only_our_own_suffix() {
    let suffix = rust_i18n::t!("common.duplicate_name", name = "").to_string();
    // 单层副本与"副本叠副本"都回到原始名。
    assert_eq!(
        duplicate_base(&format!("web-01{suffix}")),
        "web-01".to_string()
    );
    assert_eq!(
        duplicate_base(&format!("web-01{suffix} #2")),
        "web-01".to_string()
    );
    // 用户自己的命名绝不改动：没有我们的后缀时，连尾部的 #N 也不剥。
    assert_eq!(duplicate_base("db #12"), "db #12".to_string());
    assert_eq!(duplicate_base("web-01"), "web-01".to_string());
    assert_eq!(
        duplicate_base("release #12 #13"),
        "release #12 #13".to_string()
    );
    // 只剥一次我们的后缀，不做递归猜测。
    assert_eq!(strip_copy_suffix(&format!("a{suffix}"), &suffix), Some("a"));
    assert_eq!(strip_copy_suffix("plain", &suffix), None);
}
