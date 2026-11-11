//! IntelliJ New UI 领域图标。

use std::borrow::Cow;

use gpui_kit::SharedString;
use gpui_kit::component::IconNamed;
use rust_embed::RustEmbed;

/// 领域图标在资产源中的目录。
pub const DOMAIN_ICON_DIR: &str = "icons/domains";

#[derive(RustEmbed)]
#[folder = "assets/icons/domains"]
struct DomainAssets;

/// 一个领域图标；只保留 Transom 实际使用的 SSH、终端、容器、监控与转发资产。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DomainIcon(&'static str);

impl DomainIcon {
    pub const CONTAINER: Self = Self("container");
    pub const FORWARD: Self = Self("forward");
    pub const MONITOR: Self = Self("monitor");
    pub const SSH: Self = Self("ssh");
    pub const TERMINAL: Self = Self("terminal");

    /// 当前图标的浅色资产路径。
    pub fn light_path(self) -> SharedString {
        format!("{DOMAIN_ICON_DIR}/{}.svg", self.0).into()
    }
}

impl IconNamed for DomainIcon {
    fn path(self) -> SharedString {
        self.light_path()
    }
}

pub fn asset_data(path: &str) -> Option<Cow<'static, [u8]>> {
    let relative = path.strip_prefix(DOMAIN_ICON_DIR)?.strip_prefix('/')?;
    DomainAssets::get(relative).map(|asset| asset.data)
}

pub fn asset_paths() -> impl Iterator<Item = SharedString> {
    DomainAssets::iter()
        .filter(|name| name.ends_with(".svg"))
        .map(|name| format!("{DOMAIN_ICON_DIR}/{name}").into())
}

pub fn dark_variant(path: &str) -> Option<SharedString> {
    let relative = path.strip_prefix(DOMAIN_ICON_DIR)?.strip_prefix('/')?;
    let stem = relative.strip_suffix(".svg")?;
    if stem.ends_with("_dark") {
        return None;
    }
    let dark_file = format!("{stem}_dark.svg");
    DomainAssets::get(&dark_file).map(|_| format!("{DOMAIN_ICON_DIR}/{dark_file}").into())
}

#[cfg(test)]
mod tests {
    use super::{DomainIcon, asset_data, dark_variant};

    #[test]
    fn domain_assets_and_dark_variants_exist() {
        assert!(asset_data("icons/domains/container.svg").is_some());
        assert!(asset_data("icons/domains/container_dark.svg").is_some());
        assert_eq!(
            dark_variant("icons/domains/container.svg").as_deref(),
            Some("icons/domains/container_dark.svg")
        );
        assert_eq!(
            DomainIcon::CONTAINER.light_path().as_ref(),
            "icons/domains/container.svg"
        );
        assert_eq!(
            DomainIcon::TERMINAL.light_path().as_ref(),
            "icons/domains/terminal.svg"
        );
    }
}
