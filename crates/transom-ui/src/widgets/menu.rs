//! 右键菜单项的统一样式入口。

use gpui_kit::assets::IconName;
use gpui_kit::component::menu::PopupMenuItem;
use gpui_kit::*;

/// 构造一个标准菜单项：图标 + 文案 + 点击回调。
///
/// 调用方仍负责把业务动作适配成 GPUI 回调；这里只消除每个菜单项都手写一遍
/// `.icon(...).on_click(...)` 的重复，并保证图标与文案顺序一致。
pub(crate) fn menu_item(
    label: impl Into<SharedString>,
    icon: IconName,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> PopupMenuItem {
    PopupMenuItem::new(label.into().to_string())
        .icon(icon)
        .on_click(on_click)
}
