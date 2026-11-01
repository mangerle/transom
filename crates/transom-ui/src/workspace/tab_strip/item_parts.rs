//! 标签条目的徽标与关闭按钮等子元素。

use super::*;

/// 渲染分屏窗格列表徽标。
pub(super) fn render_pane_badge(
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
            let tab = shell.state.workspace.tab(pane_id)?;
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
                                .child(crate::widgets::status_dot_bare(dot_color))
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
                    menu = menu
                        .item(PopupMenuItem::separator())
                        .item(crate::widgets::menu_item(
                            rust_i18n::t!("menu.restore_panes").to_string(),
                            IconName::Minimize2,
                            {
                                let app = app_entity.clone();
                                move |_, _window, cx| {
                                    app.update(cx, |this, cx| {
                                        this.toggle_zoom_pane(active_pane, cx)
                                    });
                                }
                            },
                        ));
                }
                menu
            }
        })
        .into_any_element()
}

/// 渲染标签关闭按钮（支持并排组下拉或单标签直接关闭）。
pub(super) fn render_tab_close_button(
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
                menu.item(crate::widgets::menu_item(
                    rust_i18n::t!("menu.close_tab").to_string(),
                    IconName::X,
                    move |_, window, cx| {
                        close_current.update(cx, |this, cx| {
                            this.close_group(group_id, window, cx);
                        });
                    },
                ))
                .item(crate::widgets::menu_item(
                    rust_i18n::t!("menu.close_tiling_group").to_string(),
                    IconName::Trash,
                    move |_, window, cx| {
                        close_all.update(cx, |this, cx| {
                            this.close_tiling_group(tiling_id, window, cx);
                        });
                    },
                ))
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
