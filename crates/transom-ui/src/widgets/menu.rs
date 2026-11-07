//! 右键菜单项的统一样式入口。

use crate::app_assets::AppIcon;
use crate::theme::tokens::{space, typography};
use gpui_kit::base::h_flex;
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::menu::PopupMenuItem;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

/// 菜单项构建参数：图标、文案与可选禁用态。
///
/// 只覆盖“图标 + 文案 + 可选禁用”这一类菜单项；需要绑定真实 `Action` 的条目
/// 仍直接用 `PopupMenuItem`，因为 `Action` 自带键位展示，套一层构造器反而
/// 会藏掉组件库对动作的关联逻辑。
pub(crate) struct MenuItem {
    label: String,
    icon: AppIcon,
    shortcut: Option<String>,
    disabled: bool,
}

impl MenuItem {
    pub(crate) fn new(label: impl Into<SharedString>, icon: impl Into<AppIcon>) -> Self {
        Self {
            label: label.into().to_string(),
            icon: icon.into(),
            shortcut: None,
            disabled: false,
        }
    }

    /// 快捷键提示（如 "Ctrl+D"、"F4" 等），在右侧以弱色文字展示。
    pub(crate) fn shortcut(mut self, shortcut: impl Into<String>) -> Self {
        self.shortcut = Some(shortcut.into());
        self
    }

    /// 置灰但保留：让用户知道功能存在，而不是在当前上下文里凭空消失。
    pub(crate) fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// 生成菜单项；若携带快捷键则使用 Element 自定义两端对齐布局，禁用项不挂点击回调。
    pub(crate) fn render(
        self,
        on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> PopupMenuItem {
        let label = self.label;
        let shortcut = self.shortcut;
        let item = if shortcut.is_some() {
            PopupMenuItem::element(move |_window, cx| {
                h_flex()
                    .w_full()
                    .items_center()
                    .justify_between()
                    .gap(rems(space::LG))
                    .child(label.clone())
                    .when_some(shortcut.clone(), |el, s| {
                        el.child(
                            div()
                                .text_size(rems(typography::CAPTION))
                                .text_color(cx.theme().muted_foreground.opacity(0.65))
                                .child(s),
                        )
                    })
            })
            .icon(self.icon)
            .disabled(self.disabled)
        } else {
            PopupMenuItem::new(label)
                .icon(self.icon)
                .disabled(self.disabled)
        };

        if self.disabled {
            item
        } else {
            item.on_click(on_click)
        }
    }
}

/// 构造一个标准菜单项：图标 + 文案 + 点击回调。
pub(crate) fn menu_item(
    label: impl Into<SharedString>,
    icon: impl Into<AppIcon>,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> PopupMenuItem {
    MenuItem::new(label, icon).render(on_click)
}

/// 构造一个带右侧快捷键提示的菜单项。
pub(crate) fn menu_item_with_shortcut(
    label: impl Into<SharedString>,
    icon: impl Into<AppIcon>,
    shortcut: impl Into<String>,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> PopupMenuItem {
    MenuItem::new(label, icon)
        .shortcut(shortcut)
        .render(on_click)
}
