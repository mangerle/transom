//! 本地化资源变更需要触发本 crate 重新编译。
//!
//! rust-i18n 的 `i18n!` 宏在编译期把 locales 目录的内容打包进二进制，
//! 但不会自动向 Cargo 声明文件依赖；不显式声明时，只改 ui.yml 不会重建
//! 终端 crate，界面会出现"主界面已换文案、终端菜单还是旧文案"的半更新。

fn main() {
    println!("cargo:rerun-if-changed=../transom-ui/locales");
}
