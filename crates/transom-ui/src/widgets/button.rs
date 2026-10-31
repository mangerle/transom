//! 按钮类表现组件：图标按钮与弹窗动作按钮。
//!
//! 只统一“长什么样”，不接管“点了做什么”。回调仍由调用方提供，因此同一套
//! 按钮可以用于页面、工具栏、弹窗与测试，不会把 `AppShell` 的更新逻辑带进来。

use gpui_kit::assets::IconName;
use gpui_kit::component::Disableable;
use gpui_kit::component::Selectable;
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::*;

/// 应用图标按钮的构建参数。
pub(crate) struct IconButton {
    id: ElementId,
    icon: IconName,
    tooltip: SharedString,
    selected: bool,
    disabled: bool,
}

impl IconButton {
    /// 创建图标按钮；默认使用 ghost + xsmall，保持工具栏与侧栏的视觉一致。
    pub(crate) fn new(
        id: impl Into<ElementId>,
        icon: IconName,
        tooltip: impl Into<SharedString>,
    ) -> Self {
        Self {
            id: id.into(),
            icon,
            tooltip: tooltip.into(),
            selected: false,
            disabled: false,
        }
    }

    /// 选中态：用于隐藏文件、同步导航、筛选等开关型按钮。
    pub(crate) fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// 禁用态：保留按钮占位，但不接受点击。
    pub(crate) fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// 生成底层 `Button`；调用方继续链式接 `.on_click(...)`。
    pub(crate) fn render(self) -> Button {
        Button::new(self.id)
            .ghost()
            .xsmall()
            .icon(self.icon)
            .tooltip(self.tooltip)
            .selected(self.selected)
            .disabled(self.disabled)
    }
}

/// 常用图标按钮的快捷构造函数。
pub(crate) fn icon_button(
    id: impl Into<ElementId>,
    icon: IconName,
    tooltip: impl Into<SharedString>,
) -> Button {
    IconButton::new(id, icon, tooltip).render()
}

/// 弹窗取消按钮：统一的 ghost 样式与默认文案。
pub(crate) fn dialog_cancel_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Button {
    Button::new(id)
        .ghost()
        .label(label.into())
        .on_click(on_click)
}

/// 弹窗主按钮的语义档位。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DialogConfirmKind {
    /// 普通提交（保存、确认）。
    Primary,
    /// 破坏性提交（删除、强制启动）。
    Danger,
}

/// 弹窗确认按钮：统一主/危险样式与可选图标。
pub(crate) fn dialog_confirm_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    icon: Option<IconName>,
    kind: DialogConfirmKind,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Button {
    let button = match kind {
        DialogConfirmKind::Primary => Button::new(id).primary(),
        DialogConfirmKind::Danger => Button::new(id).danger(),
    };
    let button = match icon {
        Some(icon) => button.icon(icon),
        None => button,
    };
    button.label(label.into()).on_click(on_click)
}
