//! 单个标签条目 UI 组件组装与子元素渲染。

use crate::app::{AppShell, TabGroup};
use crate::divider::{DividerEdge, DividerStyled};
use crate::palette::Palette;
use crate::theme::tokens::size;
use crate::tiling::{MAX_TILING_SLOTS, StripEntry, TiledTabs};
use gpui_kit::assets::IconName;
use gpui_kit::base::h_flex;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{ContextMenuExt, DropdownMenu, PopupMenuItem};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;
use transom_core::{Connection, GroupId, SplitAxis, TabId};

#[path = "item_parts.rs"]
mod item_parts;

use self::item_parts::{render_pane_badge, render_tab_close_button};
use super::badges::{
    PaneRow, build_tab_tooltip, build_tiled_group_badge, calculate_dot_color, connection_icon,
};
use super::menu::{TabContextMenuParams, build_tab_context_menu};
use crate::connection::session_host;
use crate::workspace::tab_drag::attach_tab_drag;

/// 渲染标签栏中的单个条目（可以是单标签或并排组）。
pub(super) fn render_tab_entry(
    shell: &AppShell,
    entry: &StripEntry,
    can_tile: bool,
    p: &Palette,
    app_entity: &Entity<AppShell>,
    cx: &mut Context<AppShell>,
) -> Option<AnyElement> {
    let (group_id, composite, tiled_set) = match entry {
        StripEntry::Single(group) => (*group, false, None),
        StripEntry::Tiled { id, focus } => {
            let tiled = shell.tiling_by_id(*id)?.clone();
            (*focus, true, Some(tiled))
        }
    };

    let group = shell.workspace.group(group_id)?;
    let tile_enabled = !composite && can_tile;
    let tab = shell
        .workspace
        .tabs
        .iter()
        .find(|tab| tab.id == group.active_pane)?;

    let palette = p.clone();
    let group_id = group.id;
    let active_pane = group.active_pane;
    let session_id = tab.session_id;
    let title = tab.tab_label();
    let pane_count = group.pane_count();
    let root_axis = group.root.root_axis();
    let serial = group.serial;
    let is_extra = serial > 0;
    let is_local = matches!(tab.session.connection, Connection::Local(_));
    let active = shell.workspace.active_tab == Some(active_pane);
    let tiled_paired = composite;
    // 底边由条目内的绝对定位色条绘制，内边距预留同一高度，保证各条目
    // 的内容区一致，不会因活动条目的强调线更粗而上下错位。
    let divider_px = p.divider_width.pixels();
    let accent_px = divider_px.max(2.0);

    let selection_groups = match &tiled_set {
        Some(tiled) => tiled.occupied_groups(),
        None => vec![group_id],
    };
    let entry_selected = !selection_groups.is_empty()
        && selection_groups
            .iter()
            .all(|group| shell.workspace.tab_selection.contains(group));

    let selected_singles: Vec<GroupId> = shell
        .workspace
        .groups
        .iter()
        .filter(|group| {
            shell.workspace.tab_selection.contains(&group.id)
                && shell.tiling_of_group(group.id).is_none()
        })
        .map(|group| group.id)
        .collect();

    let touched_sets: Vec<crate::tiling::TilingId> = shell
        .workspace
        .tiled_sets
        .iter()
        .filter(|tiled| {
            tiled
                .occupied_groups()
                .iter()
                .any(|group| shell.workspace.tab_selection.contains(group))
        })
        .map(|tiled| tiled.id)
        .collect();

    let this_set_id = tiled_set.as_ref().map(|tiled| tiled.id);
    let show_add_to_tiling = match (this_set_id, touched_sets.as_slice()) {
        (Some(id), [only]) if *only == id => !selected_singles.is_empty(),
        _ => false,
    };
    let show_multi_tile = entry_selected
        && touched_sets.is_empty()
        && (2..=MAX_TILING_SLOTS).contains(&selected_singles.len());

    let dot_color = calculate_dot_color(tab, p);
    let base_tab_icon = if tab.is_player() {
        IconName::Disc
    } else if tab.is_monitor() {
        IconName::Gauge
    } else {
        connection_icon(&tab.session.connection)
    };

    let appearance = shell.session_appearance(tab);
    let tab_icon = if tab.is_player() {
        IconName::Disc
    } else if tab.is_monitor() {
        IconName::Gauge
    } else {
        appearance.icon.unwrap_or(base_tab_icon)
    };
    let session_color = appearance.color;

    let mut tab_element = h_flex()
        .id(ElementId::Name(format!("tab-{group_id}").into()))
        .relative()
        .h_full()
        .px_2p5()
        .pb(px(divider_px))
        .gap_1p5()
        .items_center()
        .max_w(rems(size::TAB_MAX_WIDTH))
        .min_w(rems(size::TAB_MIN_WIDTH))
        .overflow_hidden()
        .border_color(p.divider(p.border))
        .cursor_pointer();

    if active {
        tab_element = tab_element
            .bg(p.tab_active)
            .text_color(p.tab_active_foreground);
    } else {
        tab_element = tab_element.text_color(p.tab_foreground).hover(|style| {
            style
                .bg(p.sidebar_accent.opacity(0.6))
                .text_color(p.foreground)
        });
    }

    // 标签之间的竖向分隔线跟随分割线设置；放在状态分支之后，避免各分支重写。
    tab_element = tab_element.divider_edge(DividerEdge::Right, p.divider_width);

    let labeled = if is_extra {
        format!("{title} #{serial}")
    } else {
        title.clone()
    };
    let tooltip_title = if pane_count > 1 {
        rust_i18n::t!("tab.pane_count", title = labeled, count = pane_count).to_string()
    } else {
        labeled
    };
    let session_name = tab.display_name();
    let show_session = !session_name.is_empty() && session_name != title;

    let tab_element = if composite {
        tab_element
    } else {
        attach_tab_drag(
            tab_element,
            group_id,
            title.clone(),
            tab_icon,
            app_entity,
            p,
        )
    };

    let menu_app_entity = app_entity.clone();
    let menu_selection = selected_singles.clone();
    let touched_sets_count = touched_sets.len();
    let selected_singles_count = selected_singles.len();
    let menu_host = session_host(&tab.session).map(str::to_string);

    let mut element = tab_element
        .tooltip(build_tab_tooltip(
            tooltip_title,
            session_name,
            show_session,
            p.muted_foreground,
        ))
        .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
            let modifiers = event.modifiers();
            if modifiers.control || modifiers.platform {
                this.toggle_tab_selection(&selection_groups, cx);
            } else {
                this.clear_tab_selection(cx);
                this.activate_tab(active_pane, window, cx);
            }
        }))
        .context_menu(move |menu, window, cx| {
            build_tab_context_menu(
                menu,
                window,
                cx,
                &menu_app_entity,
                TabContextMenuParams {
                    active_pane,
                    session_id,
                    is_local,
                    pane_count,
                    tile_enabled,
                    group_id,
                    this_set_id,
                    show_add_to_tiling,
                    menu_selection: menu_selection.clone(),
                    show_multi_tile,
                    palette: palette.clone(),
                    entry_selected,
                    touched_sets_count,
                    selected_singles_count,
                    host: menu_host.clone(),
                },
            )
        });

    if let Some(color) = session_color {
        element = element.child(
            div()
                .absolute()
                .left_0()
                .top_0()
                .bottom_0()
                .w(px(3.0))
                .bg(color),
        );
    }

    element = element
        .child(div().size(px(8.0)).rounded_full().flex_none().bg(dot_color))
        .child(Icon::new(tab_icon).flex_none().text_color(if active {
            p.tab_active_foreground
        } else {
            p.muted_foreground
        }))
        .child(
            div()
                .text_sm()
                .flex_1()
                .overflow_hidden()
                .whitespace_nowrap()
                .truncate()
                .child(title.clone()),
        );

    if is_extra {
        element = element.child(
            div()
                .text_xs()
                .flex_none()
                .text_color(p.muted_foreground)
                .child(format!("#{serial}")),
        );
    }

    if pane_count > 1 && !composite {
        let pane_badge = render_pane_badge(shell, group, active_pane, root_axis, app_entity, p);
        element = element.child(pane_badge);
    }

    if let Some(tiled) = &tiled_set {
        element = element.child(build_tiled_group_badge(shell, tiled, app_entity, p));
    }

    if shell.settings.window.show_tab_close_button {
        let close_btn =
            render_tab_close_button(group_id, active, tiled_set.as_ref(), app_entity, p, cx);
        element = element.child(close_btn);
    }

    // 标签栏底边按条目分段：活动条目用强调色顶替自己那段，并排配对条目用
    // 弱化色，其余用分割线色。强调线不低于 2px，避免发丝档下被压低消失。
    let (bar_height, bar_color) = if active {
        (accent_px, p.primary)
    } else if tiled_paired {
        (divider_px, p.muted_foreground.opacity(0.6))
    } else {
        (divider_px, p.divider(p.border))
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

    if entry_selected {
        element = element.child(
            div()
                .absolute()
                .inset_0()
                .border_1()
                .border_color(p.primary)
                .rounded(p.radius.sm),
        );
    }

    Some(element.into_any_element())
}
