//! 应用自有 SVG 资产与图标名。
//!
//! 设计原理（Why）：`gpui-kit` 的完整图标目录不包含 Docker 品牌图标，而应用
//! 又需要与通用容器图标区分。这里在默认资产源之上只追加一个受限路径，避免
//! 复制整套依赖资产，也避免用普通形状冒充品牌标志。

use gpui_kit::assets::{AllAssets, IconName};
use gpui_kit::component::IconNamed;
use gpui_kit::*;
use std::borrow::Cow;

/// 应用自有 Docker 品牌图标在资产源中的路径。
pub(crate) const DOCKER_ICON_PATH: &str = "icons/docker.svg";

/// 应用层图标：默认使用 `gpui-kit` 图标，Docker 相关入口使用品牌图标。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AppIcon {
    Named(IconName),
    Docker,
}

impl From<IconName> for AppIcon {
    fn from(icon: IconName) -> Self {
        Self::Named(icon)
    }
}

impl IconNamed for AppIcon {
    fn path(self) -> SharedString {
        match self {
            Self::Named(icon) => icon.path(),
            Self::Docker => DOCKER_ICON_PATH.into(),
        }
    }
}

/// 组合默认图标库与应用自有 SVG 的资产源。
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if path == DOCKER_ICON_PATH {
            return Ok(Some(Cow::Borrowed(include_bytes!(
                "../assets/icons/docker.svg"
            ))));
        }
        AllAssets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = AllAssets.list(path)?;
        if DOCKER_ICON_PATH.starts_with(path) {
            paths.push(DOCKER_ICON_PATH.into());
        }
        Ok(paths)
    }
}

#[cfg(test)]
mod tests {
    use super::{AppAssets, AssetSource, DOCKER_ICON_PATH};

    #[test]
    fn app_assets_serves_docker_icon_and_delegates_builtin_icons() {
        let assets = AppAssets;
        let docker = assets
            .load(DOCKER_ICON_PATH)
            .expect("读取 Docker 图标")
            .expect("图标存在");
        assert!(std::str::from_utf8(&docker).is_ok_and(|svg| svg.contains("viewBox")));
        assert!(
            assets
                .load("icons/search.svg")
                .expect("读取内置图标")
                .is_some()
        );
        assert!(
            assets
                .list("icons/docker")
                .expect("列出 Docker 图标")
                .iter()
                .any(|path| path.as_ref() == DOCKER_ICON_PATH)
        );
    }
}
