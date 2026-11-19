//! 纵向标签面板的行渲染与交互。
//!
//! 每组一行：状态点、连接图标、会话名（含 `#N` 序号）、主机副标题，
//! 以及分屏/并排角标与关闭按钮。点击激活该组的活动窗格，中键关闭组，
//! 右键复用横向标签栏的上下文菜单，拖拽上下换序由 `drag` 模块处理。

use super::drag::attach_vertical_tab_drag;
use crate::app::app_settings;
use crate::app::{AppShell, AppState, SessionMenuData};
use crate::palette::Palette;
use crate::theme::tokens::size;

use crate::workspace::tab_strip::menu::{TabContextMenuParams, build_tab_context_menu};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::menu::ContextMenuExt;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::*;
use transom_core::{GroupId, SplitAxis};
use transom_ui_icons::AppIcon;
use transom_ui_icons::AppIconElement;
use transom_ui_icons::UiIcon;

/// 行槽位高度；与侧栏会话行保持一致。
const ROW_HEIGHT: Rems = rems(size::TREE_ROW_HEIGHT);
/// 内层高亮条高度：留出上下间隙，背景不覆盖整行槽位。
const ROW_BAND_HEIGHT: Rems = rems(size::TREE_BAND_HEIGHT);
/// 行内图标尺寸：与侧栏树行图标一致，随界面密度同步缩放。
const ROW_ICON_SIZE: Rems = rems(size::TREE_ROW_ICON);

/// 渲染纵向标签面板中的单个标签组行。
pub(super) fn render_vertical_tab_row(
    app_state: &AppState,
    group_id: GroupId,
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> Option<AnyElement> {
    let data = collect_row_data(app_state, group_id, p, cx)?;
    let tiled = app_state.tiling_of_group(group_id);
    let app_entity = cx.entity().clone();
    let mut band = build_tab_band(&data, p);
    if app_settings(cx).window.show_tab_close_button {
        band = band.child(crate::workspace::tab_strip::render_tab_close_button(
            group_id,
            data.active,
            tiled,
            &app_entity,
            p,
            cx,
        ));
    }
    let row = attach_vertical_tab_drag(
        div()
            .id(ElementId::Name(
                format!("vertical-tab-row-{group_id}").into(),
            ))
            .w_full()
            .h(ROW_HEIGHT)
            .flex()
            .items_center(),
        group_id,
        data.title.clone(),
        data.icon,
        &app_entity,
        p,
    );
    Some(apply_row_events(row, band, &data, app_entity, p, cx).into_any_element())
}

/// 行渲染所需的数据快照：只由状态推导，不再在渲染期回读实体。
struct VerticalTabRowData {
    group_id: GroupId,
    active_pane: transom_core::TabId,
    session_menu: Option<SessionMenuData>,
    can_split: bool,
    pane_count: usize,
    pane_axis: SplitAxis,
    active: bool,
    entry_selected: bool,
    title: String,
    labeled: String,
    subtitle: String,
    dot_color: gpui_kit::Hsla,
    icon: AppIcon,
    icon_color: gpui_kit::Hsla,
    tiled_badge: bool,
    this_set_id: Option<crate::tiling::TilingId>,
    can_tile: bool,
    selection_groups: Vec<GroupId>,
    selected_singles: Vec<GroupId>,
    touched_sets_count: usize,
    selected_singles_count: usize,
    show_add_to_tiling: bool,
    show_multi_tile: bool,
    tooltip_label: String,
}

fn collect_row_data(
    app_state: &AppState,
    group_id: GroupId,
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> Option<VerticalTabRowData> {
    let group = app_state.workspace.group(group_id)?;
    let tab = app_state.workspace.tab(group.active_pane)?;
    let active_pane = group.active_pane;
    let pane_count = group.pane_count();
    let pane_axis = group.root.root_axis();
    // 激活判定用"组包含活动窗格"而不是比较活动窗格本身：组内切窗格后，
    // 行仍然保持高亮，不会短暂熄灭。
    let active = app_state
        .workspace
        .active_tab
        .is_some_and(|tab_id| group.contains(tab_id));
    let title = tab.tab_label();
    let presentation = tab.presentation(app_state, group.serial, active, p, cx);
    let labeled = presentation.title.clone();
    let subtitle = presentation.subtitle.clone();
    let dot_color = presentation.status_color;
    let (icon, icon_color) = (presentation.icon, presentation.icon_color);
    let session_menu = presentation.session_menu.clone();
    let can_split = presentation.can_split;
    let tiled = app_state.tiling_of_group(group_id);
    let tiled_badge = tiled.is_some();
    let this_set_id = tiled.map(|tiled| tiled.id);
    let can_tile = app_state.workspace.groups.len() >= 2;
    let crate::workspace::tab_selection::TabSelectionModel {
        selection_groups,
        entry_selected,
        selected_singles,
        touched_sets,
        show_add_to_tiling,
        show_multi_tile,
    } = crate::workspace::tab_selection::selection_model(&app_state.workspace, group_id, tiled);
    let selected_singles_count = selected_singles.len();
    let touched_sets_count = touched_sets.len();
    let tooltip_label = if subtitle.is_empty() {
        labeled.clone()
    } else {
        format!("{labeled}\n{subtitle}")
    };
    Some(VerticalTabRowData {
        group_id,
        active_pane,
        session_menu: session_menu.clone(),
        can_split,
        pane_count,
        pane_axis,
        active,
        entry_selected,
        title,
        labeled,
        subtitle,
        dot_color,
        icon,
        icon_color,
        tiled_badge,
        this_set_id,
        can_tile,
        selection_groups,
        selected_singles,
        touched_sets_count,
        selected_singles_count,
        show_add_to_tiling,
        show_multi_tile,
        tooltip_label,
    })
}

fn build_tab_band(data: &VerticalTabRowData, p: &Palette) -> Stateful<Div> {
    let mut band = h_flex()
        .id(ElementId::Name(
            format!("vertical-tab-{}", data.group_id).into(),
        ))
        .relative()
        .w_full()
        .h(ROW_BAND_HEIGHT)
        .px_2()
        .gap_2()
        .items_center()
        .rounded(p.radius.md)
        .bg(if data.active {
            p.sidebar_accent
        } else if data.entry_selected {
            p.sidebar_accent.opacity(0.42)
        } else {
            p.sidebar_accent.opacity(0.14)
        })
        .text_color(if data.active {
            p.sidebar_accent_foreground
        } else {
            p.sidebar_foreground
        })
        .cursor_pointer();
    if data.active {
        // 活动行与悬浮态同为实心强调色，靠左侧竖条区分"当前正在使用的组"。
        band = band.child(
            div()
                .absolute()
                .left_0()
                .top_0()
                .bottom_0()
                .w(px(2.0))
                .rounded_full()
                .bg(p.list_active_border),
        );
    } else {
        band = band.hover(|style| style.bg(p.sidebar_accent));
    }
    let band = band
        .child(crate::widgets::status_dot_bare(data.dot_color))
        .child(
            AppIconElement::new(data.icon)
                .with_rems(ROW_ICON_SIZE)
                .text_color(data.icon_color),
        )
        .child(tab_band_labels(data, p));
    add_tab_band_badges(band, data, p)
}

fn tab_band_labels(data: &VerticalTabRowData, p: &Palette) -> Div {
    v_flex()
        .flex_1()
        .min_w_0()
        .child(
            div()
                .text_sm()
                .line_height(px(17.0))
                .font_weight(FontWeight::MEDIUM)
                .truncate()
                .child(data.labeled.clone()),
        )
        .child(
            div()
                .text_xs()
                .line_height(px(14.0))
                .truncate()
                .text_color(if data.active {
                    p.sidebar_accent_foreground
                } else {
                    p.muted_foreground
                })
                .child(data.subtitle.clone()),
        )
}

fn add_tab_band_badges(
    band: Stateful<Div>,
    data: &VerticalTabRowData,
    p: &Palette,
) -> Stateful<Div> {
    let mut band = band;
    if data.pane_count > 1 {
        band = band.child(split_count_badge(data, p));
    }
    if data.tiled_badge {
        band = band.child(
            AppIconElement::new(AppIcon::Ui(UiIcon::EDITOR_PREVIEW))
                .xsmall()
                .text_color(if data.active {
                    p.sidebar_accent_foreground
                } else {
                    p.muted_foreground
                }),
        );
    }
    band
}

fn split_count_badge(data: &VerticalTabRowData, p: &Palette) -> AnyElement {
    let split_icon = if data.pane_axis == SplitAxis::Horizontal {
        AppIcon::Ui(UiIcon::EDITOR_PREVIEW)
    } else {
        AppIcon::Ui(UiIcon::EDITOR_ONLY)
    };
    h_flex()
        .flex_none()
        .gap_0p5()
        .items_center()
        .text_color(if data.active {
            p.sidebar_accent_foreground
        } else {
            p.muted_foreground
        })
        .child(AppIconElement::new(split_icon).xsmall())
        .child(div().text_xs().child(data.pane_count.to_string()))
        .into_any_element()
}

fn apply_row_events(
    row: Stateful<Div>,
    band: Stateful<Div>,
    data: &VerticalTabRowData,
    app_entity: Entity<AppShell>,
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> Stateful<Div> {
    let group_id = data.group_id;
    let active_pane = data.active_pane;
    let selection_groups = data.selection_groups.clone();
    let tooltip_label = data.tooltip_label.clone();
    let menu_active_pane = data.active_pane;
    let menu_session_menu = data.session_menu.clone();
    let menu_pane_count = data.pane_count;
    let menu_tile_enabled = data.this_set_id.is_none() && data.can_tile;
    let menu_group_id = data.group_id;
    let menu_this_set_id = data.this_set_id;
    let menu_show_add_to_tiling = data.show_add_to_tiling;
    let menu_selection = data.selected_singles.clone();
    let menu_show_multi_tile = data.show_multi_tile;
    let menu_palette = p.clone();
    let menu_entry_selected = data.entry_selected;
    let menu_touched_sets_count = data.touched_sets_count;
    let menu_selected_singles_count = data.selected_singles_count;
    let menu_can_split = data.can_split;
    row.child(
        band.on_mouse_down(
            MouseButton::Middle,
            cx.listener(move |this, _, window, cx| {
                this.close_group(group_id, window, cx);
            }),
        )
        .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
            let modifiers = event.modifiers();
            if modifiers.control || modifiers.platform {
                crate::tiling::toggle_tab_selection(&mut this.state, &selection_groups, cx);
            } else {
                crate::tiling::clear_tab_selection(&mut this.state, cx);
                this.activate_tab(active_pane, window, cx);
            }
        }))
        .context_menu(move |menu, window, cx| {
            build_tab_context_menu(
                menu,
                window,
                cx,
                &app_entity,
                TabContextMenuParams {
                    active_pane: menu_active_pane,
                    session_menu: menu_session_menu.clone(),
                    can_split: menu_can_split,
                    pane_count: menu_pane_count,
                    tile_enabled: menu_tile_enabled,
                    group_id: menu_group_id,
                    this_set_id: menu_this_set_id,
                    show_add_to_tiling: menu_show_add_to_tiling,
                    menu_selection: menu_selection.clone(),
                    show_multi_tile: menu_show_multi_tile,
                    palette: menu_palette.clone(),
                    entry_selected: menu_entry_selected,
                    touched_sets_count: menu_touched_sets_count,
                    selected_singles_count: menu_selected_singles_count,
                },
            )
        }),
    )
    .tooltip(move |_window, cx| Tooltip::new(tooltip_label.clone()).build(_window, cx))
}
