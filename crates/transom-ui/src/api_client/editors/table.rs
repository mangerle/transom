//! API 表格编辑器共用的行状态管理与网格渲染原语。
//!
//! # 设计原理（Why）
//! `kv`、`form`、`environment`、`cookie`、`assertion` 五个表格编辑器共享同一套
//! “末行保留空白输入行 + 勾选用行 + 网格表头/单元格 + 删除按钮”交互模式。
//! 将通用骨架收敛到本模块，可避免在五个文件中重复维护高度同构的布局与增删逻辑。

use crate::theme::tokens::{size, space, typography};
use crate::widgets::IconButton;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_ui_icons::{AppIcon, UiIcon};

/// 表格列宽规格。
#[derive(Clone, Copy)]
pub(in crate::api_client) enum ColumnWidth {
    Fixed(Rems),
    Relative(f32),
    Flex,
}

/// 按列宽规格配置容器尺寸。
fn apply_column_width(div: Div, width: ColumnWidth) -> Div {
    match width {
        ColumnWidth::Fixed(w) => div.w(w).flex_none(),
        ColumnWidth::Relative(ratio) => div.w(relative(ratio)).flex_none(),
        ColumnWidth::Flex => div.flex_1(),
    }
}

/// 渲染表格外层带边框与竖向滚动的统一容器。
pub(super) fn table_scroll_frame(
    id: &'static str,
    border: Hsla,
    header: AnyElement,
    rows: impl IntoIterator<Item = AnyElement>,
) -> AnyElement {
    v_flex()
        .id(id)
        .flex_1()
        .min_h_0()
        .w_full()
        .p(rems(space::SM))
        .child(
            div()
                .flex_1()
                .min_h_0()
                .w_full()
                .overflow_y_scrollbar()
                .rounded(rems(space::XS))
                .border_1()
                .border_color(border)
                .overflow_hidden()
                .child(header)
                .children(rows),
        )
        .into_any_element()
}

/// 渲染统一风格的表格表头栏（含可选首尾操作列占位）。
pub(in crate::api_client) fn table_header_row(
    border: Hsla,
    bg: Hsla,
    muted_foreground: Hsla,
    has_leading: bool,
    has_trailing: bool,
    columns: Vec<(String, ColumnWidth)>,
) -> AnyElement {
    let mut row = h_flex()
        .h(rems(size::LARGE_HEADER_HEIGHT))
        .bg(bg)
        .text_size(rems(typography::CAPTION))
        .text_color(muted_foreground)
        .border_color(border)
        .border_b_1();
    if has_leading {
        row = row.child(div().w(rems(3.0)).flex_none());
    }
    for (label, width) in columns {
        row = row.child(table_header_cell(label, width, border));
    }
    if has_trailing {
        row = row.child(div().w(rems(3.0)).flex_none());
    }
    row.into_any_element()
}

/// 渲染单个表头单元格。
pub(in crate::api_client) fn table_header_cell(
    label: String,
    width: ColumnWidth,
    border: Hsla,
) -> AnyElement {
    apply_column_width(div(), width)
        .min_w_0()
        .h_full()
        .px(rems(space::SM))
        .flex()
        .items_center()
        .border_l_1()
        .border_color(border)
        .child(label)
        .into_any_element()
}

/// 渲染普通文本输入框单元格。
pub(super) fn table_input_cell(
    input: &Entity<InputState>,
    width: ColumnWidth,
    border: Hsla,
) -> AnyElement {
    apply_column_width(div(), width)
        .min_w_0()
        .h_full()
        .flex()
        .items_center()
        .px(rems(space::SM))
        .border_l_1()
        .border_color(border)
        .child(Input::new(input).small().appearance(false))
        .into_any_element()
}

/// 渲染居中复选框单元格。
pub(super) fn table_checkbox_cell(
    key: &'static str,
    id: u64,
    checked: bool,
    width: ColumnWidth,
    border: Hsla,
    on_click: impl Fn(&bool, &mut Window, &mut App) + 'static,
) -> AnyElement {
    apply_column_width(div(), width)
        .h_full()
        .flex()
        .items_center()
        .justify_center()
        .border_l_1()
        .border_color(border)
        .child(Checkbox::new((key, id)).checked(checked).on_click(on_click))
        .into_any_element()
}

/// 渲染自定义内容单元格（带统一左边框与高度）。
pub(super) fn table_custom_cell(
    width: ColumnWidth,
    border: Hsla,
    child: impl IntoElement,
) -> AnyElement {
    apply_column_width(div(), width)
        .min_w_0()
        .h_full()
        .flex()
        .items_center()
        .px(rems(space::SM))
        .border_l_1()
        .border_color(border)
        .child(child)
        .into_any_element()
}

/// 渲染首尾固定 3rem 宽度的操作槽位。
pub(super) fn table_action_slot(child: Option<AnyElement>) -> AnyElement {
    div()
        .w(rems(3.0))
        .flex_none()
        .h_full()
        .flex()
        .items_center()
        .justify_center()
        .when_some(child, |slot, child| slot.child(child))
        .into_any_element()
}

/// 渲染行首“+”添加按钮。
pub(super) fn table_add_button(
    key: &'static str,
    id: u64,
    tooltip: String,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    IconButton::new((key, id), AppIcon::Ui(UiIcon::ADD), tooltip)
        .render()
        .on_click(on_click)
        .into_any_element()
}

/// 渲染行尾删除按钮。
pub(super) fn table_remove_button(
    key: &'static str,
    id: u64,
    tooltip: String,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    IconButton::new((key, id), AppIcon::Ui(UiIcon::DELETE), tooltip)
        .render()
        .on_click(on_click)
        .into_any_element()
}

pub(super) type TableAddCallback = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;
pub(super) type TableToggleCallback = Box<dyn Fn(&bool, &mut Window, &mut App) + 'static>;

/// 行首标准启用/添加槽位的渲染参数。
pub(super) struct LeadingToggleSlotParams {
    pub(super) add_key: &'static str,
    pub(super) toggle_key: &'static str,
    pub(super) id: u64,
    pub(super) blank: bool,
    pub(super) enabled: bool,
    pub(super) add_tooltip: String,
    pub(super) on_add: TableAddCallback,
    pub(super) on_toggle: TableToggleCallback,
}

/// 渲染行首标准启用/添加槽位（空白行显示添加按钮，已有行显示复选框）。
pub(super) fn table_leading_toggle_slot(params: LeadingToggleSlotParams) -> AnyElement {
    let child = if params.blank {
        table_add_button(params.add_key, params.id, params.add_tooltip, params.on_add)
    } else {
        Checkbox::new((params.toggle_key, params.id))
            .checked(params.enabled)
            .on_click(params.on_toggle)
            .into_any_element()
    };
    table_action_slot(Some(child))
}

/// 渲染统一高度与底部分割线的表格行容器。
pub(super) fn table_row_container(
    key: &'static str,
    id: u64,
    dimmed: bool,
    border: Hsla,
) -> Stateful<Div> {
    h_flex()
        .id((key, id))
        .h(rems(size::LARGE_HEADER_HEIGHT))
        .items_center()
        .border_color(border)
        .border_b_1()
        .when(dimmed, |row| row.opacity(0.45))
}
