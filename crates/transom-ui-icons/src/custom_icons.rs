//! Transom 自有领域图标。
//!
//! 设计原理（Why）：IntelliJ New UI 没有为磁盘、数据卷、端口等业务语义提供
//! 完全匹配的通用图标，继续借用 SSH、数据库或内存图标会让界面含义漂移。这里
//! 单独维护项目自有 SVG，与 IntelliJ 资产目录和许可记录分开。
//!
//! 权衡：自有图标需要随设计调整同步维护明暗两套；但换来了稳定的业务语义，
//! 后续升级 IntelliJ 资产时也不会覆盖这些图标。
//!
//! 所有描边统一使用 1.5px 光学线宽，避免 1px 或小数线宽在 14px 紧凑尺寸下
//! 被抗锯齿冲淡；Docker、Podman 使用单色界面外形，避免高饱和品牌色在侧栏
//! 和标签栏中形成视觉突兀。

use std::borrow::Cow;

use gpui_kit::SharedString;
use gpui_kit::component::IconNamed;
use rust_embed::RustEmbed;

/// 自有图标在资产源中的目录。
pub const CUSTOM_ICON_DIR: &str = "icons/custom";

#[derive(RustEmbed)]
#[folder = "assets/icons/custom"]
struct CustomAssets;

/// 一个 Transom 自有领域图标。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CustomIcon(&'static str);

impl CustomIcon {
    pub const NETWORK: Self = Self("network");
    pub const CPU: Self = Self("cpu");
    pub const DATABASE: Self = Self("database");
    pub const CLOUD: Self = Self("cloud");
    pub const MEMORY: Self = Self("memory");
    pub const DISK: Self = Self("disk");
    pub const OS: Self = Self("os");
    pub const VOLUME: Self = Self("volume");
    pub const MOUNT: Self = Self("mount");
    pub const PORT: Self = Self("port");
    pub const PROCESS: Self = Self("process");
    pub const IMAGE: Self = Self("image");
    pub const SNIPPET: Self = Self("snippet");
    pub const BROADCAST: Self = Self("broadcast");
    pub const RECORDING: Self = Self("recording");
    pub const DOCKER: Self = Self("docker");
    pub const PODMAN: Self = Self("podman");
    pub const INFO: Self = Self("info");
    pub const CONNECTION_TEST: Self = Self("connection_test");
    pub const FINGERPRINT: Self = Self("fingerprint");
    pub const EYE_OFF: Self = Self("eye_off");
    pub const PANEL_WIDEN: Self = Self("panel_widen");
    pub const PANEL_NARROW: Self = Self("panel_narrow");

    /// 当前图标的浅色资产路径。
    pub fn light_path(self) -> SharedString {
        format!("{CUSTOM_ICON_DIR}/{}.svg", self.0).into()
    }
}

impl IconNamed for CustomIcon {
    fn path(self) -> SharedString {
        self.light_path()
    }
}

/// 从嵌入资产读取指定自有图标。
pub fn asset_data(path: &str) -> Option<Cow<'static, [u8]>> {
    let relative = path.strip_prefix(CUSTOM_ICON_DIR)?.strip_prefix('/')?;
    CustomAssets::get(relative).map(|asset| asset.data)
}

/// 列出全部自有图标资产。
pub fn asset_paths() -> impl Iterator<Item = SharedString> {
    CustomAssets::iter()
        .filter(|name| name.ends_with(".svg"))
        .map(|name| format!("{CUSTOM_ICON_DIR}/{name}").into())
}

/// 若路径是自有图标的浅色版本且有深色资产，返回深色路径。
pub fn dark_variant(path: &str) -> Option<SharedString> {
    let relative = path.strip_prefix(CUSTOM_ICON_DIR)?.strip_prefix('/')?;
    let stem = relative.strip_suffix(".svg")?;
    if stem.ends_with("_dark") {
        return None;
    }
    let dark_file = format!("{stem}_dark.svg");
    CustomAssets::get(&dark_file).map(|_| format!("{CUSTOM_ICON_DIR}/{dark_file}").into())
}

#[cfg(test)]
mod tests {
    use super::{CustomIcon, asset_data, dark_variant};

    #[test]
    fn custom_assets_and_dark_variants_exist() {
        for icon in [
            CustomIcon::NETWORK,
            CustomIcon::CPU,
            CustomIcon::DATABASE,
            CustomIcon::CLOUD,
            CustomIcon::MEMORY,
            CustomIcon::DISK,
            CustomIcon::OS,
            CustomIcon::VOLUME,
            CustomIcon::MOUNT,
            CustomIcon::PORT,
            CustomIcon::PROCESS,
            CustomIcon::IMAGE,
            CustomIcon::SNIPPET,
            CustomIcon::BROADCAST,
            CustomIcon::RECORDING,
            CustomIcon::DOCKER,
            CustomIcon::PODMAN,
            CustomIcon::INFO,
            CustomIcon::CONNECTION_TEST,
            CustomIcon::FINGERPRINT,
            CustomIcon::EYE_OFF,
            CustomIcon::PANEL_WIDEN,
            CustomIcon::PANEL_NARROW,
        ] {
            let light = icon.light_path();
            assert!(asset_data(light.as_ref()).is_some());
            assert!(dark_variant(light.as_ref()).is_some());
        }
    }

    #[test]
    fn custom_strokes_use_consistent_optical_width() {
        for path in super::asset_paths() {
            let data = asset_data(path.as_ref()).expect("自有图标资产必须存在");
            let svg = std::str::from_utf8(data.as_ref()).expect("自有图标必须是 UTF-8 SVG");
            if svg.contains("stroke=") {
                assert!(
                    svg.contains("stroke-width=\"1.5\""),
                    "自有图标 {path} 未使用统一的 1.5px 描边"
                );
                assert_eq!(
                    svg.matches("stroke-width=").count(),
                    1,
                    "自有图标 {path} 存在覆盖根节点线宽的局部描边"
                );
            }
        }
    }
}
