//! 单个标签条目 UI 组件组装与子元素渲染。
//!
//! 设计原理（Why）：标签条目同时包含状态徽标、拖拽、右键菜单、并排徽标和底边
//! 状态条。先把一次性解析结果收进 [`TabEntryModel`]，再按“基础布局、交互、
//! 内容、装饰”四段装配，避免单个渲染函数继续堆叠闭包。

use super::badges::{build_tab_tooltip, build_tiled_group_badge};
use super::item_parts::{render_pane_badge, render_tab_close_button};
use super::menu::{TabContextMenuParams, build_tab_context_menu};
use crate::app::{AppShell, TabGroup, TerminalTab, app_settings};
use crate::connection::session_host;
use crate::divider::{DividerEdge, DividerStyled};
use crate::palette::Palette;
use crate::theme::tokens::size;
use crate::tiling::{StripEntry, TiledTabs};
use crate::workspace::WorkspaceState;
use crate::workspace::tab_drag::attach_tab_drag;
use crate::workspace::tab_selection::TabSelectionModel;
use gpui_kit::base::{Tab, h_flex};
use gpui_kit::component::menu::ContextMenuExt;
use gpui_kit::*;
use transom_core::{Connection, GroupId, SessionId, SplitAxis, TabId};
use transom_ui_icons::{AppIcon, AppIconElement};

/// 单个标签条目的渲染参数包，避免七个平铺入参发生顺序错位。
pub(super) struct TabEntryRenderParams<'model, 'cx_ref, 'ctx> {
    pub(super) model: TabEntryModelParams<'model>,
    pub(super) position: usize,
    pub(super) size: usize,
    pub(super) cx: &'cx_ref mut Context<'ctx, AppShell>,
}

/// 只服务一次标签条目解析的参数包。
pub(super) struct TabEntryModelParams<'a> {
    pub(super) shell: &'a AppShell,
    pub(super) entry: &'a StripEntry,
    pub(super) can_tile: bool,
    pub(super) palette: &'a Palette,
    pub(super) app: &'a Entity<AppShell>,
}

/// 标签条目在单标签与并排组两种形态下的身份解析结果。
struct TabEntryTarget {
    group_id: GroupId,
    composite: bool,
    tiled_set: Option<TiledTabs>,
}

/// 标签条目展示尺寸与状态标志。
struct TabEntryDisplayState {
    tile_enabled: bool,
    active: bool,
    pane_count: usize,
    root_axis: SplitAxis,
    serial: usize,
    is_local: bool,
    divider_px: f32,
    accent_px: f32,
}

/// 标签标题、提示与会话副标题的统一口径。
struct TabLabels {
    title: String,
    tooltip_title: String,
    session_name: String,
    show_session: bool,
}

/// 标签图标、状态点与会话强调色。
struct TabVisuals {
    icon: AppIcon,
    dot_color: Hsla,
    session_color: Option<Hsla>,
}

/// 标签条目一次性解析出的布局、状态与交互数据。
struct TabEntryModel<'a> {
    group_id: GroupId,
    active_pane: TabId,
    session_id: SessionId,
    group: &'a TabGroup,
    tab: &'a TerminalTab,
    tiled_set: Option<TiledTabs>,
    this_set_id: Option<crate::tiling::TilingId>,
    composite: bool,
    selection: TabSelectionModel,
    display: TabEntryDisplayState,
    labels: TabLabels,
    visuals: TabVisuals,
    workspace: &'a WorkspaceState,
    palette: &'a Palette,
    app: &'a Entity<AppShell>,
}

/// 渲染标签栏中的单个条目（可以是单标签或并排组）。
pub(super) fn render_tab_entry<'model, 'cx_ref, 'ctx>(
    params: TabEntryRenderParams<'model, 'cx_ref, 'ctx>,
) -> Option<AnyElement> {
    let TabEntryRenderParams {
        model,
        position,
        size,
        cx,
    } = params;
    let model = resolve_tab_entry_model(model, cx)?;
    let element = build_tab_entry_base(&model);
    let element = attach_tab_entry_interactions(element, &model);
    let element = append_tab_entry_content(element, &model);
    let element = append_tab_entry_chrome(element, &model, cx);
    let tab = build_tab_entry_tab(element, &model, position, size, cx);
    let menu_params = build_tab_context_menu_params(&model);
    let menu_app = model.app.clone();
    Some(
        tab.context_menu(move |menu, window, cx| {
            build_tab_context_menu(menu, window, cx, &menu_app, menu_params.clone())
        })
        .into_any_element(),
    )
}

/// 解析标签、分组、并排关系、焦点与展示状态。
fn resolve_tab_entry_model<'a>(
    params: TabEntryModelParams<'a>,
    cx: &mut Context<'_, AppShell>,
) -> Option<TabEntryModel<'a>> {
    let TabEntryModelParams {
        shell,
        entry,
        can_tile,
        palette,
        app,
    } = params;
    let TabEntryTarget {
        group_id,
        composite,
        tiled_set,
    } = resolve_tab_entry_target(|id| shell.state.tiling_by_id(id).cloned(), entry)?;
    let group = shell.state.workspace.group(group_id)?;
    let tab = shell
        .state
        .workspace
        .tabs
        .iter()
        .find(|tab| tab.id == group.active_pane)?;
    let selection = crate::workspace::tab_selection::selection_model(
        &shell.state.workspace,
        group_id,
        tiled_set.as_ref(),
    );
    let active_pane = group.active_pane;
    let display = resolve_tab_display_state(
        &shell.state.workspace,
        group,
        tab,
        composite,
        can_tile,
        palette,
    );
    let labels = resolve_tab_labels(tab, display.serial, display.pane_count);
    let visuals = resolve_tab_visuals(&shell.state, tab, display.active, palette, cx);
    Some(TabEntryModel {
        group_id,
        active_pane,
        session_id: tab.session_id,
        group,
        tab,
        this_set_id: tiled_set.as_ref().map(|tiled| tiled.id),
        tiled_set,
        composite,
        selection,
        display,
        labels,
        visuals,
        workspace: &shell.state.workspace,
        palette,
        app,
    })
}

/// 解析标签展示状态、尺寸与可用动作。
fn resolve_tab_display_state(
    workspace: &WorkspaceState,
    group: &TabGroup,
    tab: &TerminalTab,
    composite: bool,
    can_tile: bool,
    palette: &Palette,
) -> TabEntryDisplayState {
    let divider_px = palette.divider_width.pixels();
    TabEntryDisplayState {
        tile_enabled: !composite && can_tile,
        active: workspace.active_tab == Some(group.active_pane),
        pane_count: group.pane_count(),
        root_axis: group.root.root_axis(),
        serial: group.serial,
        is_local: matches!(tab.session.connection, Connection::Local(_)),
        divider_px,
        accent_px: divider_px.max(2.0),
    }
}

/// 解析单标签或并排组对应的目标分组。
fn resolve_tab_entry_target(
    tiling_by_id: impl FnOnce(crate::tiling::TilingId) -> Option<TiledTabs>,
    entry: &StripEntry,
) -> Option<TabEntryTarget> {
    match entry {
        StripEntry::Single(group) => Some(TabEntryTarget {
            group_id: *group,
            composite: false,
            tiled_set: None,
        }),
        StripEntry::Tiled { id, focus } => Some(TabEntryTarget {
            group_id: *focus,
            composite: true,
            tiled_set: Some(tiling_by_id(*id)?),
        }),
    }
}

/// 统一计算标签标题、提示文本与会话副标题。
fn resolve_tab_labels(tab: &TerminalTab, serial: usize, pane_count: usize) -> TabLabels {
    let title = tab.tab_label();
    let labeled = crate::workspace::tab_presentation::tab_title(tab, serial);
    let session_name = tab.display_name();
    let tooltip_title = if pane_count > 1 {
        rust_i18n::t!("tab.pane_count", title = labeled, count = pane_count).to_string()
    } else {
        labeled
    };
    TabLabels {
        show_session: !session_name.is_empty() && session_name != title,
        title,
        tooltip_title,
        session_name,
    }
}

/// 统一计算标签图标、状态点与会话强调色。
fn resolve_tab_visuals(
    state: &crate::app::AppState,
    tab: &TerminalTab,
    active: bool,
    palette: &Palette,
    cx: &App,
) -> TabVisuals {
    TabVisuals {
        icon: crate::workspace::tab_presentation::tab_icon(state, tab, active, palette, cx).0,
        dot_color: crate::workspace::tab_presentation::tab_status_dot(tab, palette),
        session_color: crate::workspace::tab_presentation::tab_accent(state, tab, cx),
    }
}

/// 构建标签条目的基础尺寸与颜色状态。
///
/// 页签角色、焦点与键盘激活由外层 `Tab` 负责；这里只保留鼠标命中区域、
/// 拖拽句柄和视觉状态，避免内部再声明一份会与 roving focus 冲突的 Tab 语义。
fn build_tab_entry_base(model: &TabEntryModel<'_>) -> Stateful<Div> {
    let p = model.palette;
    let mut element = h_flex()
        .id(ElementId::Name(format!("tab-{}", model.group_id).into()))
        .relative()
        .h_full()
        .px_2p5()
        .pb(px(model.display.divider_px))
        .gap_1p5()
        .items_center()
        .max_w(rems(size::TAB_MAX_WIDTH))
        .min_w(rems(size::TAB_MIN_WIDTH))
        .overflow_hidden()
        .border_color(p.divider(p.border))
        .cursor_pointer();
    if model.display.active {
        element = element.bg(p.tab_active).text_color(p.tab_active_foreground);
    } else {
        element = element.text_color(p.tab_foreground).hover(|style| {
            style
                .bg(p.sidebar_accent.opacity(0.6))
                .text_color(p.foreground)
        });
    }
    element.divider_edge(DividerEdge::Right, p.divider_width)
}

/// 挂载拖拽与提示交互。
fn attach_tab_entry_interactions(
    element: Stateful<Div>,
    model: &TabEntryModel<'_>,
) -> Stateful<Div> {
    let element = if model.composite {
        element
    } else {
        attach_tab_drag(
            element,
            model.group_id,
            model.labels.title.clone(),
            model.visuals.icon,
            model.app,
            model.palette,
        )
    };
    element.tooltip(build_tab_tooltip(
        model.labels.tooltip_title.clone(),
        model.labels.session_name.clone(),
        model.labels.show_session,
        model.palette.muted_foreground,
    ))
}

/// 用外层 `Tab` 包裹视觉条目，统一页签角色、焦点和激活行为。
fn build_tab_entry_tab(
    element: Stateful<Div>,
    model: &TabEntryModel<'_>,
    position: usize,
    size: usize,
    cx: &mut Context<AppShell>,
) -> Tab {
    let active_pane = model.active_pane;
    let selection_groups = model.selection.selection_groups.clone();
    Tab::new(ElementId::Name(format!("tab-{}", model.group_id).into()))
        .h_full()
        .flex_none()
        .cursor_pointer()
        .selected(model.display.active)
        .accessibility_label(model.labels.title.clone())
        .set_position(position, size)
        .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
            let modifiers = event.modifiers();
            if modifiers.control || modifiers.platform {
                crate::tiling::toggle_tab_selection(&mut this.state, &selection_groups, cx);
            } else {
                crate::tiling::clear_tab_selection(&mut this.state, cx);
                this.activate_tab(active_pane, window, cx);
            }
        }))
        .child(element)
}

/// 组装右键菜单参数快照。
fn build_tab_context_menu_params(model: &TabEntryModel<'_>) -> TabContextMenuParams {
    TabContextMenuParams {
        active_pane: model.active_pane,
        session_id: model.session_id,
        is_local: model.display.is_local,
        pane_count: model.display.pane_count,
        tile_enabled: model.display.tile_enabled,
        group_id: model.group_id,
        this_set_id: model.this_set_id,
        show_add_to_tiling: model.selection.show_add_to_tiling,
        menu_selection: model.selection.selected_singles.clone(),
        show_multi_tile: model.selection.show_multi_tile,
        palette: model.palette.clone(),
        entry_selected: model.selection.entry_selected,
        touched_sets_count: model.selection.touched_sets.len(),
        selected_singles_count: model.selection.selected_singles.len(),
        host: session_host(&model.tab.session).map(str::to_string),
    }
}

/// 追加标签强调线、图标与标题。
fn append_tab_entry_content(element: Stateful<Div>, model: &TabEntryModel<'_>) -> Stateful<Div> {
    let p = model.palette;
    let mut element = if let Some(color) = model.visuals.session_color {
        element.child(
            div()
                .absolute()
                .left_0()
                .top_0()
                .bottom_0()
                .w(px(3.0))
                .bg(color),
        )
    } else {
        element
    };
    element = element
        .child(crate::widgets::status_dot_bare(model.visuals.dot_color))
        .child(
            AppIconElement::new(model.visuals.icon)
                .flex_none()
                .text_color(if model.display.active {
                    p.tab_active_foreground
                } else {
                    p.muted_foreground
                }),
        )
        .child(
            div()
                .text_sm()
                .flex_1()
                .overflow_hidden()
                .whitespace_nowrap()
                .truncate()
                .child(model.labels.title.clone()),
        );
    if model.display.serial > 0 {
        element = element.child(
            div()
                .text_xs()
                .flex_none()
                .text_color(p.muted_foreground)
                .child(format!("#{}", model.display.serial)),
        );
    }
    element
}

/// 追加窗格与并排徽标。
fn append_tab_entry_badges(element: Stateful<Div>, model: &TabEntryModel<'_>) -> Stateful<Div> {
    let mut element = element;
    if model.display.pane_count > 1 && !model.composite {
        element = element.child(render_pane_badge(
            model.workspace,
            model.group,
            model.active_pane,
            model.display.root_axis,
            model.app,
            model.palette,
        ));
    }
    if let Some(tiled) = &model.tiled_set {
        element = element.child(build_tiled_group_badge(
            model.workspace,
            tiled,
            model.app,
            model.palette,
        ));
    }
    element
}

/// 追加关闭按钮、底边状态条与选中描边。
fn append_tab_entry_footer(
    element: Stateful<Div>,
    model: &TabEntryModel<'_>,
    cx: &mut Context<AppShell>,
) -> Stateful<Div> {
    let p = model.palette;
    let mut element = if app_settings(cx).window.show_tab_close_button {
        element.child(render_tab_close_button(
            model.group_id,
            model.display.active,
            model.tiled_set.as_ref(),
            model.app,
            p,
            cx,
        ))
    } else {
        element
    };
    let (bar_height, bar_color) = if model.display.active {
        (model.display.accent_px, p.primary)
    } else if model.composite {
        (model.display.divider_px, p.muted_foreground.opacity(0.6))
    } else {
        (model.display.divider_px, p.divider(p.border))
    };
    element = element.child(
        div()
            .absolute()
            .bottom_0()
            .left_0()
            .right_0()
            .h(px(bar_height))
            .bg(bar_color),
    );
    if model.selection.entry_selected {
        element = element.child(
            div()
                .absolute()
                .inset_0()
                .border_1()
                .border_color(p.primary)
                .rounded(p.radius.sm),
        );
    }
    element
}

/// 追加标签条目的徽标与底部状态装饰。
fn append_tab_entry_chrome(
    element: Stateful<Div>,
    model: &TabEntryModel<'_>,
    cx: &mut Context<AppShell>,
) -> Stateful<Div> {
    append_tab_entry_footer(append_tab_entry_badges(element, model), model, cx)
}
