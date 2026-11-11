//! 应用资产源：默认图标库 + `transom-ui-icons` 独立图标包。

use gpui_kit::assets::AllAssets;
use gpui_kit::*;

/// 组合默认图标库与独立 IntelliJ 图标包。
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<std::borrow::Cow<'static, [u8]>>> {
        if let Some(data) = transom_ui_icons::asset_data(path) {
            return Ok(Some(data));
        }
        AllAssets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = AllAssets.list(path)?;
        paths.extend(transom_ui_icons::asset_paths());
        Ok(paths)
    }
}

/// 安装主题感知的 IntelliJ 图标解析器。
pub(crate) fn install_icon_resolver(cx: &mut App) {
    transom_ui_icons::install_icon_resolver(cx);
}

#[cfg(test)]
mod tests {
    use super::{AppAssets, AssetSource};

    #[test]
    fn app_assets_include_every_icon_category() {
        let assets = AppAssets;
        for path in [
            "icons/custom/docker.svg",
            "icons/file-types/rust.svg",
            "icons/domains/ssh.svg",
            "icons/ui/settings.svg",
        ] {
            assert!(
                assets.load(path).expect("读取图标资产失败").is_some(),
                "缺少资产: {path}"
            );
        }
        assert!(
            assets
                .load("icons/search.svg")
                .expect("读取默认 Lucide 资产失败")
                .is_some()
        );
    }
}
