//! 标签栏右键上下文菜单构建。

use crate::app::AppShell;
use crate::palette::Palette;
use crate::tiling::{MAX_TILING_SLOTS, TilingId};
use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::*;
use transom_core::{Connection, GroupId, SplitAxis, TabId};

/// 标签条目右键上下文菜单的配置参数对象。
pub(crate) struct TabContextMenuParams {
    pub(crate) active_pane: TabId,
    pub(crate) session_id: transom_core::SessionId,
    pub(crate) is_local: bool,
    pub(crate) pane_count: usize,
    pub(crate) tile_enabled: bool,
    pub(crate) group_id: GroupId,
    pub(crate) this_set_id: Option<TilingId>,
    pub(crate) show_add_to_tiling: bool,
    pub(crate) menu_selection: Vec<GroupId>,
    pub(crate) show_multi_tile: bool,
    pub(crate) palette: Palette,
    pub(crate) entry_selected: bool,
    pub(crate) touched_sets_count: usize,
    pub(crate) selected_singles_count: usize,
    /// 会话对端主机地址；本地终端与串口为 `None`，此时不渲染"复制主机地址"项。
    pub(crate) host: Option<String>,
}

/// 构建标签条目的右键上下文菜单。
pub(crate) fn build_tab_context_menu(
    menu: PopupMenu,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
    app_entity: &Entity<AppShell>,
    params: TabContextMenuParams,
) -> PopupMenu {
    let TabContextMenuParams {
        active_pane,
        session_id,
        is_local,
        pane_count,
        tile_enabled,
        group_id,
        this_set_id,
        show_add_to_tiling,
        menu_selection,
        show_multi_tile,
        palette,
        entry_selected,
        touched_sets_count,
        selected_singles_count,
        host,
    } = params;
    let app = app_entity.clone();
    let mut menu = menu
        .item(crate::widgets::menu_item(
            rust_i18n::t!("menu.reconnect").to_string(),
            IconName::RefreshCw,
            {
                let app = app.clone();
                move |_, window, cx| {
                    app.update(cx, |this, cx| {
                        this.reconnect_tab(active_pane, window, cx);
                    });
                }
            },
        ))
        .item(crate::widgets::menu_item(
            rust_i18n::t!("menu.duplicate").to_string(),
            IconName::Plus,
            {
                let app = app.clone();
                move |_, window, cx| {
                    app.update(cx, |this, cx| {
                        this.activate_tab(active_pane, window, cx);
                        if is_local {
                            this.open_local_terminal(window, cx);
                        } else {
                            this.connect_session(session_id, window, cx);
                        }
                    });
                }
            },
        ))
        .item(crate::widgets::menu_item(
            rust_i18n::t!("audit_viewer.menu_item").to_string(),
            IconName::FileText,
            {
                let app = app.clone();
                move |_, window, cx| {
                    app.update(cx, |this, cx| {
                        this.open_audit_viewer_for_tab(active_pane, window, cx);
                    });
                }
            },
        ));

    // 只有真有对端的会话才给这一项：本地终端与串口连地址都没有，宁可不显示，
    // 也不给一个复制出空串的按钮。
    if let Some(host) = host {
        let copy_app = app.clone();
        menu = menu.item(crate::widgets::menu_item(
            rust_i18n::t!("menu.copy_host").to_string(),
            IconName::Copy,
            move |_, _, cx| {
                let text = host.clone();
                cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                copy_app.update(cx, |this, cx| {
                    this.state.status =
                        Some(rust_i18n::t!("menu.copied_host", host = text).to_string());
                    cx.notify();
                });
            },
        ));
    }

    // 若会话为 SSH 连接，提供一键直通其远端 Docker 工作区的捷径
    let is_ssh = app_entity
        .read(cx)
        .state
        .catalog
        .sessions
        .iter()
        .any(|s| s.id == session_id && matches!(s.connection, Connection::Ssh(_)));
    if is_ssh {
        let container_app = app.clone();
        menu = menu.item(crate::widgets::menu_item(
            rust_i18n::t!("menu.manage_containers").to_string(),
            IconName::Boxes,
            move |_, window, cx| {
                container_app.update(cx, |this, cx| {
                    this.open_container_workspace_for_session(session_id, window, cx);
                });
            },
        ));
    }

    menu = menu
        .item(crate::widgets::menu_item(
            rust_i18n::t!("recorder.toggle_recording").to_string(),
            IconName::Disc,
            {
                let app = app.clone();
                move |_, window, cx| {
                    app.update(cx, |this, cx| {
                        this.toggle_recording(active_pane, window, cx);
                    });
                }
            },
        ))
        .item(crate::widgets::menu_item(
            rust_i18n::t!("recorder.replay_menu").to_string(),
            IconName::Play,
            {
                let app = app.clone();
                move |_, window, cx| {
                    app.update(cx, |this, cx| {
                        this.open_replay_modal(active_pane, window, cx);
                    });
                }
            },
        ))
        .item(PopupMenuItem::separator())
        .item(crate::widgets::menu_item(
            rust_i18n::t!("menu.split_right").to_string(),
            IconName::Columns2,
            {
                let app = app.clone();
                move |_, window, cx| {
                    app.update(cx, |this, cx| {
                        this.activate_tab(active_pane, window, cx);
                        this.split_active_pane(SplitAxis::Horizontal, window, cx);
                    });
                }
            },
        ))
        .item(crate::widgets::menu_item(
            rust_i18n::t!("menu.split_down").to_string(),
            IconName::Rows2,
            {
                let app = app.clone();
                move |_, window, cx| {
                    app.update(cx, |this, cx| {
                        this.activate_tab(active_pane, window, cx);
                        this.split_active_pane(SplitAxis::Vertical, window, cx);
                    });
                }
            },
        ));
    if pane_count > 1 {
        menu = menu
            .item(crate::widgets::menu_item(
                rust_i18n::t!("menu.toggle_split_axis").to_string(),
                IconName::ArrowLeftRight,
                {
                    let app = app.clone();
                    move |_, window, cx| {
                        app.update(cx, |this, cx| {
                            this.activate_tab(active_pane, window, cx);
                            this.toggle_split_axis(cx);
                        });
                    }
                },
            ))
            .item(crate::widgets::menu_item(
                rust_i18n::t!("menu.detach_pane").to_string(),
                IconName::ArrowUpRight,
                {
                    let app = app.clone();
                    move |_, window, cx| {
                        app.update(cx, |this, cx| {
                            this.activate_tab(active_pane, window, cx);
                            this.detach_pane_to_tab(active_pane, window, cx);
                        });
                    }
                },
            ))
            .item(crate::widgets::menu_item(
                rust_i18n::t!("menu.close_pane").to_string(),
                IconName::X,
                {
                    let app = app.clone();
                    move |_, window, cx| {
                        app.update(cx, |this, cx| {
                            this.close_pane(active_pane, window, cx);
                        });
                    }
                },
            ));
    }
    menu = menu
        .item(PopupMenuItem::separator())
        .item(
            crate::widgets::MenuItem::new(
                rust_i18n::t!("menu.tile_right").to_string(),
                IconName::Columns2,
            )
            .disabled(!tile_enabled)
            .render({
                let app = app.clone();
                move |_, window, cx| {
                    app.update(cx, |this, cx| {
                        this.tile_tab(group_id, SplitAxis::Horizontal, window, cx);
                    });
                }
            }),
        )
        .item(
            crate::widgets::MenuItem::new(
                rust_i18n::t!("menu.tile_down").to_string(),
                IconName::Rows2,
            )
            .disabled(!tile_enabled)
            .render({
                let app = app.clone();
                move |_, window, cx| {
                    app.update(cx, |this, cx| {
                        this.tile_tab(group_id, SplitAxis::Vertical, window, cx);
                    });
                }
            }),
        );
    if show_add_to_tiling && let Some(set_id) = this_set_id {
        let groups = menu_selection.clone();
        let app = app.clone();
        menu = menu.item(crate::widgets::menu_item(
            rust_i18n::t!("menu.add_to_tiling", count = groups.len()).to_string(),
            IconName::LayoutGrid,
            move |_, window, cx| {
                app.update(cx, |this, cx| {
                    if let Some(group) = this.add_groups_to_tiling(set_id, groups.clone(), window) {
                        this.activate_group(group, window, cx);
                        this.state.workspace.tab_selection.clear();
                    } else {
                        this.state.set_error(
                            rust_i18n::t!("tiling.too_many", max = MAX_TILING_SLOTS).to_string(),
                        );
                    }
                    cx.notify();
                });
            },
        ));
    }
    if show_multi_tile {
        let groups = menu_selection.clone();
        let app = app.clone();
        let palette = palette.clone();
        menu = menu.submenu_with_icon(
            Some(Icon::new(IconName::LayoutGrid)),
            rust_i18n::t!("menu.tile_selected", count = groups.len()).to_string(),
            window,
            cx,
            move |menu, window, cx| {
                crate::workspace::tiling_menu::build_tiling_layout_submenu(
                    menu,
                    groups.clone(),
                    app.clone(),
                    palette.clone(),
                    window,
                    cx,
                )
            },
        );
    }
    if entry_selected && touched_sets_count > 1 && selected_singles_count > 0 {
        // 选中跨了多张并排网格：没有唯一落点，置灰而不是隐藏，
        // 让用户知道功能存在但当前选择不明确。
        menu = menu.item(
            crate::widgets::MenuItem::new(
                rust_i18n::t!("menu.tile_selected", count = selected_singles_count).to_string(),
                IconName::LayoutGrid,
            )
            .disabled(true)
            .render(|_, _, _| {}),
        );
    }
    menu = menu.item(
        crate::widgets::MenuItem::new(
            rust_i18n::t!("menu.untile").to_string(),
            IconName::Minimize2,
        )
        .disabled(this_set_id.is_none())
        .render({
            let app = app.clone();
            move |_, _window, cx| {
                if let Some(id) = this_set_id {
                    app.update(cx, |this, cx| this.untile_tiling(id, cx));
                }
            }
        }),
    );
    menu.item(PopupMenuItem::separator())
        .item(crate::widgets::menu_item(
            rust_i18n::t!("menu.close_tab").to_string(),
            IconName::X,
            {
                let app = app.clone();
                move |_, window, cx| {
                    app.update(cx, |this, cx| {
                        this.close_group(group_id, window, cx);
                    });
                }
            },
        ))
        .item(
            crate::widgets::MenuItem::new(
                rust_i18n::t!("menu.close_tiling_group").to_string(),
                IconName::Trash,
            )
            .disabled(this_set_id.is_none())
            .render({
                let app = app.clone();
                move |_, window, cx| {
                    if let Some(id) = this_set_id {
                        app.update(cx, |this, cx| {
                            this.close_tiling_group(id, window, cx);
                        });
                    }
                }
            }),
        )
        .item(crate::widgets::menu_item(
            rust_i18n::t!("menu.close_other_tabs").to_string(),
            IconName::Trash,
            {
                let app = app.clone();
                move |_, window, cx| {
                    app.update(cx, |this, cx| {
                        this.activate_tab(active_pane, window, cx);
                        this.close_other_tabs(active_pane, window, cx);
                    });
                }
            },
        ))
}
