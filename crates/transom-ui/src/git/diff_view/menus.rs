//! Git 差异工具条菜单：保留齿轮扩展占位，其余标准选项复用公共 `diff_pane` 控件。

use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::*;
use transom_ui_icons::UiIcon;

/// 齿轮菜单：当前还做不到的能力按 IDEA 的原条目置灰呈现。
pub(crate) fn render_settings_menu() -> AnyElement {
    Button::new("git-diff-gear")
        .ghost()
        .xsmall()
        .icon(UiIcon::SETTINGS)
        .accessibility_label(rust_i18n::t!("common.more").to_string())
        .tooltip(rust_i18n::t!("common.more").to_string())
        .dropdown_menu(move |menu, _window, _cx| {
            menu.item(
                PopupMenuItem::new(rust_i18n::t!("git.diff.align_changes").to_string())
                    .disabled(true),
            )
            .item(
                PopupMenuItem::new(rust_i18n::t!("git.diff.sync_scroll").to_string())
                    .disabled(true),
            )
            .item(
                PopupMenuItem::new(rust_i18n::t!("git.diff.open_in_window").to_string())
                    .disabled(true),
            )
            .item(
                PopupMenuItem::new(rust_i18n::t!("git.diff.all_files").to_string()).disabled(true),
            )
        })
        .into_any_element()
}
