//! 右键菜单项的统一样式入口。

use gpui_kit::assets::IconName;
use gpui_kit::component::menu::PopupMenuItem;
use gpui_kit::*;

/// 菜单项构建参数：图标、文案与可选禁用态。
///
/// 只覆盖“图标 + 文案 + 可选禁用”这一类菜单项；需要绑定真实 `Action` 的条目
/// 仍直接用 `PopupMenuItem`，因为 `Action` 自带键位展示，套一层构造器反而
/// 会藏掉组件库对动作的关联逻辑。
pub(crate) struct MenuItem {
    label: String,
    icon: IconName,
    disabled: bool,
}

impl MenuItem {
    pub(crate) fn new(label: impl Into<SharedString>, icon: IconName) -> Self {
        Self {
            label: label.into().to_string(),
            icon,
            disabled: false,
        }
    }

    /// 置灰但保留：让用户知道功能存在，而不是在当前上下文里凭空消失。
    pub(crate) fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// 生成菜单项；禁用项不挂点击回调，避免点击路径被条件分支污染。
    pub(crate) fn render(
        self,
        on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> PopupMenuItem {
        let item = PopupMenuItem::new(self.label)
            .icon(self.icon)
            .disabled(self.disabled);
        if self.disabled {
            item
        } else {
            item.on_click(on_click)
        }
    }
}

/// 构造一个标准菜单项：图标 + 文案 + 点击回调。
///
/// 调用方仍负责把业务动作适配成 GPUI 回调；这里只消除每个菜单项都手写一遍
/// `.icon(...).on_click(...)` 的重复，并保证图标与文案顺序一致。
pub(crate) fn menu_item(
    label: impl Into<SharedString>,
    icon: IconName,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> PopupMenuItem {
    MenuItem::new(label, icon).render(on_click)
}
