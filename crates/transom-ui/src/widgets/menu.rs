//! 右键菜单项的统一样式入口。

use crate::theme::tokens::{control, space};
use gpui_kit::base::h_flex;
use gpui_kit::component::menu::PopupMenuItem;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_ui_icons::{AppIcon, AppIconElement};

/// 菜单项构建参数：图标、文案与可选禁用态。
///
/// 设计原理（Why）：
/// - `PopupMenuItem::icon()` 会把 [`AppIcon`] 转成底层单色 `Icon` 并固定为 `12px`，
///   既小于工具栏的 `16px`（`control::ICON`），又会把带底色填充的图标（如文件夹）
///   整块涂黑。
/// - 改为通过 `PopupMenuItem::element()` 组合 [`AppIconElement`] 与文案，尺寸和
///   全彩 SVG 管线都与工具栏保持一致；权衡是在同一菜单中若混用左侧勾选框或带
///   `.icon()` 的子菜单项，需将勾选侧置右以避免额外左侧占位。
pub(crate) struct MenuItem {
    label: String,
    icon: AppIcon,
    disabled: bool,
}

impl MenuItem {
    pub(crate) fn new(label: impl Into<SharedString>, icon: impl Into<AppIcon>) -> Self {
        Self {
            label: label.into().to_string(),
            icon: icon.into(),
            disabled: false,
        }
    }

    /// 置灰但保留：让用户知道功能存在，而不是在当前上下文里凭空消失。
    pub(crate) fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// 生成菜单项；禁用项不挂点击回调。
    pub(crate) fn render(
        self,
        on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> PopupMenuItem {
        let label = self.label;
        let icon = self.icon;
        let disabled = self.disabled;
        let item = PopupMenuItem::element(move |_, _| {
            let icon_el = AppIconElement::new(icon)
                .with_rems(rems(control::ICON))
                .flex_none();
            h_flex()
                .w_full()
                .items_center()
                .gap(rems(space::SM))
                .child(
                    div()
                        .flex_none()
                        .flex()
                        .items_center()
                        .when(disabled, |this| this.opacity(0.5))
                        .child(icon_el),
                )
                .child(div().flex_1().whitespace_nowrap().child(label.clone()))
        })
        .disabled(disabled);

        if disabled {
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
