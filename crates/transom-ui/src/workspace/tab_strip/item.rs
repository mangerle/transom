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

    if active || tiled_paired {
        let (height, color) = if active {
            (px(2.0), p.primary)
        } else {
            (px(1.0), p.muted_foreground.opacity(0.6))
        };
        element = element.child(
            div()
                .absolute()
                .bottom_0()
                .left_0()
                .right_0()
                .h(height)
                .bg(color),
        );
    }

    Some(element.into_any_element())
}

/// 渲染分屏窗格列表徽标。
fn render_pane_badge(
    shell: &AppShell,
    group: &TabGroup,
    active_pane: TabId,
    root_axis: SplitAxis,
    app_entity: &Entity<AppShell>,
    p: &Palette,
) -> AnyElement {
    let rows: Vec<PaneRow> = group
        .pane_ids()
        .into_iter()
        .enumerate()
        .filter_map(|(position, pane_id)| {
            let tab = shell.workspace.tab(pane_id)?;
            let (dot_color, icon) = crate::workspace::pane_status_indicator(&tab.status, p);
            Some(PaneRow {
                tab_id: pane_id,
                ordinal: position + 1,
                title: tab.display_name(),
                dot_color,
                icon,
                active: pane_id == active_pane,
            })
        })
        .collect();
    let zoomed = group.zoomed.is_some();
    let muted = p.muted_foreground;
    let foreground = p.foreground;
    let badge_icon = if root_axis == SplitAxis::Horizontal {
        IconName::Columns2
    } else {
        IconName::Rows2
    };
    let group_id = group.id;
    let pane_count = group.pane_count();

    Button::new(ElementId::Name(format!("pane-list-{group_id}").into()))
        .ghost()
        .xsmall()
        .icon(badge_icon)
        .label(pane_count.to_string())
        .dropdown_menu({
            let app_entity = app_entity.clone();
            move |menu, _window, _cx| {
                let mut menu = menu;
                for row in rows.iter() {
                    let tab_id = row.tab_id;
                    let title = row.title.clone();
                    let (dot_color, icon, active, ordinal) =
                        (row.dot_color, row.icon, row.active, row.ordinal);
                    menu = menu.item(
                        PopupMenuItem::element(move |_window, _cx| {
                            let mut row = h_flex()
                                .gap_2()
                                .items_center()
                                .child(div().size(px(7.0)).rounded_full().flex_none().bg(dot_color))
                                .child(Icon::new(icon).xsmall().text_color(muted))
                                .child(
                                    div()
                                        .text_xs()
                                        .flex_none()
                                        .text_color(muted)
                                        .child(ordinal.to_string()),
                                )
                                .child(
                                    div()
                                        .text_sm()
                                        .flex_1()
                                        .overflow_hidden()
                                        .whitespace_nowrap()
                                        .truncate()
                                        .child(title.clone()),
                                );
                            if active {
                                row = row.child(
                                    Icon::new(IconName::Check).xsmall().text_color(foreground),
                                );
                            }
                            row
                        })
                        .on_click({
                            let app = app_entity.clone();
                            move |_, window, cx| {
                                app.update(cx, |this, cx| {
                                    this.select_pane(tab_id, window, cx);
                                });
                            }
                        }),
                    );
                }
                if zoomed {
                    menu = menu.item(PopupMenuItem::separator()).item(
                        PopupMenuItem::new(rust_i18n::t!("menu.restore_panes").to_string())
                            .icon(IconName::Minimize2)
                            .on_click({
                                let app = app_entity.clone();
                                move |_, _window, cx| {
                                    app.update(cx, |this, cx| {
                                        this.toggle_zoom_pane(active_pane, cx)
                                    });
                                }
                            }),
                    );
                }
                menu
            }
        })
        .into_any_element()
}

/// 渲染标签关闭按钮（支持并排组下拉或单标签直接关闭）。
fn render_tab_close_button(
    group_id: GroupId,
    active: bool,
    tiled_set: Option<&TiledTabs>,
    app_entity: &Entity<AppShell>,
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    if let Some(tiled) = tiled_set {
        let menu_app = app_entity.clone();
        let tiling_id = tiled.id;
        Button::new(ElementId::Name(format!("close-{group_id}").into()))
            .ghost()
            .xsmall()
            .icon(IconName::WindowClose)
            .tooltip(rust_i18n::t!("app.close").to_string())
            .dropdown_menu(move |menu, _window, _cx| {
                let close_current = menu_app.clone();
                let close_all = menu_app.clone();
                menu.item(
                    PopupMenuItem::new(rust_i18n::t!("menu.close_tab").to_string())
                        .icon(IconName::X)
                        .on_click(move |_, window, cx| {
                            close_current.update(cx, |this, cx| {
                                this.close_group(group_id, window, cx);
                            });
                        }),
                )
                .item(
                    PopupMenuItem::new(rust_i18n::t!("menu.close_tiling_group").to_string())
                        .icon(IconName::Trash)
                        .on_click(move |_, window, cx| {
                            close_all.update(cx, |this, cx| {
                                this.close_tiling_group(tiling_id, window, cx);
                            });
                        }),
                )
            })
            .into_any_element()
    } else {
        div()
            .id(ElementId::Name(format!("close-{group_id}").into()))
            .size(px(18.0))
            .rounded(p.radius.sm)
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .text_color(if active {
                p.tab_active_foreground
            } else {
                p.muted_foreground
            })
            .hover(|s| {
                s.bg(p.muted_foreground.opacity(0.2))
                    .text_color(p.foreground)
            })
            .active(|s| s.bg(p.muted_foreground.opacity(0.3)))
            .tooltip(move |_window, cx| {
                Tooltip::new(rust_i18n::t!("app.close").to_string()).build(_window, cx)
            })
            .on_mouse_down(MouseButton::Left, |_, window, cx| {
                window.prevent_default();
                cx.stop_propagation();
            })
            .on_click(cx.listener(move |this, _, window, cx| {
                cx.stop_propagation();
                this.close_group(group_id, window, cx);
            }))
            .child(Icon::new(IconName::WindowClose).xsmall())
            .into_any_element()
    }
}
