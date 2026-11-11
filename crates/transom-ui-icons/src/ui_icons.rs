//! IntelliJ New UI 操作图标。
//!
//! 设计原理（Why）：按钮、工具栏、导航与状态图标不能继续混用 Lucide 与
//! IntelliJ 资产。这里集中登记项目实际使用的 New UI 图标 stem，资产按明暗
//! 两套嵌入；新增图标必须先确认使用点，再把资产加入清单，避免整包复制。

use std::borrow::Cow;

use gpui_kit::SharedString;
use gpui_kit::component::IconNamed;
use rust_embed::RustEmbed;

/// New UI 图标在资产源中的目录。
pub const UI_ICON_DIR: &str = "icons/ui";

#[derive(RustEmbed)]
#[folder = "assets/icons/ui"]
struct UiAssets;

/// 一个 New UI 操作图标。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UiIcon(&'static str);

impl UiIcon {
    pub const ABORT: Self = Self("abort");
    pub const ADD: Self = Self("add");
    pub const ADD_FILE: Self = Self("addFile");
    pub const ADD_LINK: Self = Self("addLink");
    pub const AI_INTENTION: Self = Self("aiIntentionBulb");
    pub const ARROW_LEFT: Self = Self("arrowLeft");
    pub const ARROW_RIGHT: Self = Self("arrowRight");
    pub const CHECK: Self = Self("checked");
    pub const CHEVRON_DOWN: Self = Self("chevronDown");
    pub const CHEVRON_LEFT: Self = Self("chevronLeft");
    pub const CHEVRON_RIGHT: Self = Self("chevronRight");
    pub const CHEVRON_UP: Self = Self("chevronUp");
    pub const CLOSE: Self = Self("close");
    pub const COLLAPSE_ALL: Self = Self("collapseAll");
    pub const COMMIT: Self = Self("commit");
    pub const COPY: Self = Self("copy");
    pub const DATAFLOW: Self = Self("dataflow");
    pub const DOWNLOAD: Self = Self("download");
    pub const DELETE: Self = Self("delete");
    pub const DESKTOP: Self = Self("desktop");
    pub const DEPENDENCIES: Self = Self("dependencies");
    pub const DIFF: Self = Self("diff");
    pub const DOWN: Self = Self("down");
    pub const EDIT: Self = Self("edit");
    pub const EDITOR_ONLY: Self = Self("editorOnly");
    pub const EDITOR_PREVIEW: Self = Self("editorPreview");
    pub const ERROR: Self = Self("errorDialog");
    pub const EXCEPTION_ANALYZER: Self = Self("exceptionAnalyzer");
    pub const EXPAND_ALL: Self = Self("expandAll");
    pub const EXTERNAL_LINK: Self = Self("externalLink");
    pub const FETCH: Self = Self("fetch");
    pub const FILTER: Self = Self("filter");
    pub const FOLDER: Self = Self("folder");
    pub const GREEN_CHECKMARK: Self = Self("greenCheckmark");
    pub const HIGHLIGHT: Self = Self("highlighting");
    pub const HISTORY: Self = Self("history");
    pub const HIDE: Self = Self("hide");
    pub const HELP: Self = Self("help");
    pub const HOME_FOLDER: Self = Self("homeFolder");
    pub const LIGHTNING: Self = Self("lightning");
    pub const LANGUAGE: Self = Self("language");
    pub const KEYBOARD: Self = Self("keyboard");
    pub const LAYOUT: Self = Self("layout");
    pub const LEFT: Self = Self("left");
    pub const LIST_FILES: Self = Self("listFiles");
    pub const LOCKED: Self = Self("locked");
    pub const MERGE: Self = Self("merge");
    pub const MAXIMIZE: Self = Self("maximize");
    pub const MINIMIZE: Self = Self("minimize");
    pub const MORE_HORIZONTAL: Self = Self("moreHorizontal");
    pub const NEW_FOLDER: Self = Self("newFolder");
    pub const PASTE: Self = Self("paste");
    pub const PATCH: Self = Self("patch");
    pub const PAUSE: Self = Self("pause");
    pub const PROJECT: Self = Self("project");
    pub const PUSH: Self = Self("push");
    pub const REMOVE: Self = Self("remove");
    pub const REAL_INTENTION_BULB: Self = Self("realIntentionBulb");
    pub const REFRESH: Self = Self("refresh");
    pub const REFRESH_AUTO: Self = Self("refreshAuto");
    pub const RESTORE: Self = Self("restore");
    pub const RIGHT: Self = Self("right");
    pub const RUN_ANYTHING: Self = Self("runAnything");
    pub const RUN: Self = Self("run");
    pub const SAVE: Self = Self("save");
    pub const SEARCH: Self = Self("search");
    pub const SETTINGS: Self = Self("settings");
    pub const SHELVE: Self = Self("shelve");
    pub const SHOW: Self = Self("show");
    pub const SHOW_AS_TREE: Self = Self("tree");
    pub const SORT_BY: Self = Self("sortBy");
    pub const STAR: Self = Self("star");
    pub const STOP: Self = Self("stop");
    pub const TAG: Self = Self("tag");
    pub const TREE: Self = Self("tree");
    pub const UNDO: Self = Self("undo");
    pub const UPDATE: Self = Self("update");
    pub const UPLOAD: Self = Self("upload");
    pub const UP: Self = Self("up");
    pub const VCS: Self = Self("vcs");
    pub const WARNING: Self = Self("warningDialog");
    pub const WEB_SERVER: Self = Self("webServer");

    /// 当前图标的浅色资产路径。
    pub fn light_path(self) -> SharedString {
        format!("{UI_ICON_DIR}/{}.svg", self.0).into()
    }
}

impl IconNamed for UiIcon {
    fn path(self) -> SharedString {
        self.light_path()
    }
}

/// 从嵌入资产读取指定 New UI 图标。
pub fn asset_data(path: &str) -> Option<Cow<'static, [u8]>> {
    let relative = path.strip_prefix(UI_ICON_DIR)?.strip_prefix('/')?;
    UiAssets::get(relative).map(|asset| asset.data)
}

/// 列出全部 New UI 图标资产。
pub fn asset_paths() -> impl Iterator<Item = SharedString> {
    UiAssets::iter()
        .filter(|name| name.ends_with(".svg"))
        .map(|name| format!("{UI_ICON_DIR}/{name}").into())
}

/// 若路径是 New UI 图标的浅色版本且有深色资产，返回深色路径。
pub fn dark_variant(path: &str) -> Option<SharedString> {
    dark_asset_variant(path, UI_ICON_DIR)
}

fn dark_asset_variant(path: &str, dir: &str) -> Option<SharedString> {
    let relative = path.strip_prefix(dir)?.strip_prefix('/')?;
    let stem = relative.strip_suffix(".svg")?;
    if stem.ends_with("_dark") {
        return None;
    }
    let dark_file = format!("{stem}_dark.svg");
    UiAssets::get(&dark_file).map(|_| format!("{dir}/{dark_file}").into())
}

#[cfg(test)]
mod tests {
    use super::{UiIcon, asset_data, dark_variant};

    #[test]
    fn new_ui_assets_and_dark_variants_exist() {
        assert!(asset_data("icons/ui/refresh.svg").is_some());
        assert!(asset_data("icons/ui/refresh_dark.svg").is_some());
        for icon in [
            UiIcon::HELP,
            UiIcon::KEYBOARD,
            UiIcon::PROJECT,
            UiIcon::LAYOUT,
            UiIcon::DESKTOP,
            UiIcon::EXCEPTION_ANALYZER,
            UiIcon::REAL_INTENTION_BULB,
            UiIcon::REFRESH_AUTO,
        ] {
            let light = icon.light_path();
            assert!(asset_data(light.as_ref()).is_some());
            let dark = format!("{}_dark.svg", light.trim_end_matches(".svg"));
            assert!(asset_data(&dark).is_some());
        }
        assert_eq!(
            dark_variant("icons/ui/search.svg").as_deref(),
            Some("icons/ui/search_dark.svg")
        );
        assert_eq!(UiIcon::PUSH.light_path().as_ref(), "icons/ui/push.svg");
    }
}
