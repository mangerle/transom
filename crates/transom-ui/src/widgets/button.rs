//! 按钮类表现组件：图标按钮与弹窗动作按钮。
//!
//! 只统一“长什么样”，不接管“点了做什么”。回调仍由调用方提供，因此同一套
//! 按钮可以用于页面、工具栏、弹窗与测试，不会把 `AppShell` 的更新逻辑带进来。

use crate::theme::tokens::{control, space};
use gpui_kit::base::h_flex;
use gpui_kit::component::Disableable;
use gpui_kit::component::Selectable;
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::*;
use transom_ui_icons::{AppIcon, AppIconElement};

/// 应用图标按钮的构建参数。
pub(crate) struct IconButton {
    id: ElementId,
    icon: AppIcon,
    accessibility_label: SharedString,
    tooltip: Option<SharedString>,
    selected: bool,
    disabled: bool,
    size: IconButtonSize,
}

/// 图标按钮尺寸档位；统一走设计令牌，避免各页面各写 20/22/26px。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IconButtonSize {
    /// 26px：侧栏与主工具栏。
    Regular,
    /// 22px：标签栏等紧凑位置。
    Compact,
    /// 20px：列表行内动作。
    RowAction,
}

impl IconButtonSize {
    fn rem(self) -> f32 {
        match self {
            Self::Regular => control::ICON_BUTTON,
            Self::Compact => control::COMPACT_ICON_BUTTON,
            Self::RowAction => control::ROW_ACTION,
        }
    }
}

impl IconButton {
    /// 创建图标按钮；默认使用 ghost + xsmall，保持工具栏与侧栏的视觉一致。
    pub(crate) fn new(
        id: impl Into<ElementId>,
        icon: impl Into<AppIcon>,
        accessibility_label: impl Into<SharedString>,
    ) -> Self {
        let accessibility_label = accessibility_label.into();
        Self {
            id: id.into(),
            icon: icon.into(),
            tooltip: Some(accessibility_label.clone()),
            accessibility_label,
            selected: false,
            disabled: false,
            size: IconButtonSize::Regular,
        }
    }

    /// 覆盖 tooltip 文案；tooltip 是可选的视觉补充，不替代可访问名。
    pub(crate) fn tooltip(mut self, tooltip: impl Into<SharedString>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }

    /// 控制是否显示 tooltip；熟悉的重复操作可只保留可访问名。
    pub(crate) fn show_tooltip(mut self, show: bool) -> Self {
        if !show {
            self.tooltip = None;
        }
        self
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

    /// 尺寸档位：同一套视觉语义在侧栏、标签栏与列表行内复用。
    pub(crate) fn size(mut self, size: IconButtonSize) -> Self {
        self.size = size;
        self
    }

    /// 生成底层 `Button`；调用方继续链式接 `.on_click(...)`。
    pub(crate) fn render(self) -> Button {
        let size = rems(self.size.rem());
        let icon_size = match self.size {
            IconButtonSize::Regular => rems(control::ICON),
            IconButtonSize::Compact | IconButtonSize::RowAction => rems(control::COMPACT_ICON),
        };
        let mut button = Button::new(self.id)
            .ghost()
            .xsmall()
            .child(AppIconElement::new(self.icon).with_rems(icon_size))
            .accessibility_label(self.accessibility_label)
            .selected(self.selected)
            .disabled(self.disabled)
            .w(size)
            .h(size);
        if let Some(tooltip) = self.tooltip {
            button = button.tooltip(tooltip);
        }
        button
    }
}

/// 带前置图标的操作按钮：固定“图标在左 + 6px 间距 + 文字在右”与 `.small()`（24px）高度。
///
/// 设计原理（Why）：`gpui-component` 的 `Button::label()` 与 `.child()` 同用时，
/// `label` 固定排在 `children` 前面，且 `.xsmall()`（20px）塞进 16px 图标会紧贴边缘。
/// 将图标与文案装入 `h_flex` 作为唯一 `child`，即可保证图标在左、14px 图标与 24px
/// 按钮比例协调，同时保留 `.primary()` / `.ghost()` / `.danger()` / `.dropdown_menu()` 等全部能力。
pub(crate) fn action_button(
    id: impl Into<ElementId>,
    icon: impl Into<AppIcon>,
    label: impl Into<SharedString>,
) -> Button {
    action_button_with_icon_tint(id, icon, label, None)
}

/// 带前置图标的操作按钮，并在需要时强制图标使用指定单色。
///
/// 选中页签等强调态需要跨不同图标分类统一着色；普通按钮传入 `None` 时继续保留
/// IntelliJ 资产原色。
pub(crate) fn action_button_with_icon_tint(
    id: impl Into<ElementId>,
    icon: impl Into<AppIcon>,
    label: impl Into<SharedString>,
    icon_tint: Option<Hsla>,
) -> Button {
    let label = label.into();
    Button::new(id)
        .small()
        .accessibility_label(label.clone())
        .child(icon_label_content(
            icon.into(),
            rems(control::COMPACT_ICON),
            label,
            icon_tint,
        ))
}

fn icon_label_content(
    icon: AppIcon,
    icon_size: Rems,
    label: SharedString,
    icon_tint: Option<Hsla>,
) -> AnyElement {
    let mut icon = AppIconElement::new(icon).with_rems(icon_size).flex_none();
    if let Some(color) = icon_tint {
        icon = icon.monochrome(color);
    }
    h_flex()
        .items_center()
        .gap(rems(space::COMPACT))
        .child(icon)
        .child(
            div()
                .min_w_0()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(label),
        )
        .into_any_element()
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

/// 弹窗确认按钮：统一主/危险样式与可选前置图标。
pub(crate) fn dialog_confirm_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    icon: Option<AppIcon>,
    kind: DialogConfirmKind,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Button {
    let label = label.into();
    let button = match kind {
        DialogConfirmKind::Primary => Button::new(id).primary(),
        DialogConfirmKind::Danger => Button::new(id).danger(),
    };
    let button = match icon {
        Some(icon) => button
            .accessibility_label(label.clone())
            .child(icon_label_content(icon, rems(control::ICON), label, None)),
        None => button.label(label),
    };
    button.on_click(on_click)
}
