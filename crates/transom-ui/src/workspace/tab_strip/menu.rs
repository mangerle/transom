//! 标签栏右键上下文菜单构建。

use crate::app::AppShell;
use crate::palette::Palette;
use crate::tiling::{MAX_TILING_SLOTS, TilingId};
use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::*;
use transom_core::{GroupId, SplitAxis, TabId};

/// 构建标签条目的右键上下文菜单。
#[allow(clippy::too_many_arguments)]
pub(super) fn build_tab_context_menu(
    menu: PopupMenu,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
    app_entity: &Entity<AppShell>,
    active_pane: TabId,
    session_id: transom_core::SessionId,
    is_local: bool,
    pane_count: usize,
    tile_enabled: bool,
    group_id: GroupId,
    this_set_id: Option<TilingId>,
    show_add_to_tiling: bool,
    menu_selection: Vec<GroupId>,
    show_multi_tile: bool,
    palette: Palette,
    entry_selected: bool,
    touched_sets_count: usize,
    selected_singles_count: usize,
) -> PopupMenu {
    let app = app_entity.clone();
    let mut menu = menu
        .item(
            PopupMenuItem::new(rust_i18n::t!("menu.reconnect").to_string())
                .icon(IconName::RefreshCw)
                .on_click({
                    let app = app.clone();
                    move |_, window, cx| {
                        app.update(cx, |this, cx| {
                            this.reconnect_tab(active_pane, window, cx);
                        });
                    }
                }),
        )
        .item(
            PopupMenuItem::new(rust_i18n::t!("menu.duplicate").to_string())
                .icon(IconName::Plus)
                .on_click({
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
                }),
        )
        .item(
            PopupMenuItem::new(rust_i18n::t!("audit_viewer.menu_item").to_string())
                .icon(IconName::FileText)
                .on_click({
                    let app = app.clone();
                    move |_, window, cx| {
                        app.update(cx, |this, cx| {
                            this.open_audit_viewer_for_tab(active_pane, window, cx);
                        });
                    }
                }),
        )
        .item(
            PopupMenuItem::new(rust_i18n::t!("recorder.toggle_recording").to_string())
                .icon(IconName::Disc)
                .on_click({
                    let app = app.clone();
                    move |_, window, cx| {
                        app.update(cx, |this, cx| {
                            this.toggle_recording(active_pane, window, cx);
                        });
                    }
                }),
        )
        .item(
            PopupMenuItem::new(rust_i18n::t!("recorder.replay_menu").to_string())
                .icon(IconName::Play)
                .on_click({
                    let app = app.clone();
                    move |_, window, cx| {
                        app.update(cx, |this, cx| {
                            this.open_replay_modal(active_pane, window, cx);
                        });
                    }
                }),
        )
        .item(PopupMenuItem::separator())
        .item(
            PopupMenuItem::new(rust_i18n::t!("menu.split_right").to_string())
                .icon(IconName::Columns2)
                .on_click({
                    let app = app.clone();
                    move |_, window, cx| {
                        app.update(cx, |this, cx| {
                            this.activate_tab(active_pane, window, cx);
                            this.split_active_pane(SplitAxis::Horizontal, window, cx);
                        });
                    }
                }),
        )
        .item(
            PopupMenuItem::new(rust_i18n::t!("menu.split_down").to_string())
                .icon(IconName::Rows2)
                .on_click({
                    let app = app.clone();
                    move |_, window, cx| {
                        app.update(cx, |this, cx| {
                            this.activate_tab(active_pane, window, cx);
                            this.split_active_pane(SplitAxis::Vertical, window, cx);
                        });
                    }
                }),
        );
    if pane_count > 1 {
        menu = menu
            .item(
                PopupMenuItem::new(rust_i18n::t!("menu.toggle_split_axis").to_string())
                    .icon(IconName::ArrowLeftRight)
                    .on_click({
                        let app = app.clone();
                        move |_, window, cx| {
                            app.update(cx, |this, cx| {
                                this.activate_tab(active_pane, window, cx);
                                this.toggle_split_axis(cx);
                            });
                        }
                    }),
            )
            .item(
                PopupMenuItem::new(rust_i18n::t!("menu.detach_pane").to_string())
                    .icon(IconName::ArrowUpRight)
                    .on_click({
                        let app = app.clone();
                        move |_, window, cx| {
                            app.update(cx, |this, cx| {
                                this.activate_tab(active_pane, window, cx);
                                this.detach_pane_to_tab(active_pane, window, cx);
                            });
                        }
                    }),
            )
            .item(
                PopupMenuItem::new(rust_i18n::t!("menu.close_pane").to_string())
                    .icon(IconName::X)
                    .on_click({
                        let app = app.clone();
                        move |_, window, cx| {
                            app.update(cx, |this, cx| {
                                this.close_pane(active_pane, window, cx);
                            });
                        }
                    }),
            );
    }
    menu = menu
        .item(PopupMenuItem::separator())
        .item(
            PopupMenuItem::new(rust_i18n::t!("menu.tile_right").to_string())
                .icon(IconName::Columns2)
                .disabled(!tile_enabled)
                .on_click({
                    let app = app.clone();
                    move |_, window, cx| {
                        app.update(cx, |this, cx| {
                            this.tile_tab(group_id, SplitAxis::Horizontal, window, cx);
                        });
                    }
                }),
        )
        .item(
            PopupMenuItem::new(rust_i18n::t!("menu.tile_down").to_string())
                .icon(IconName::Rows2)
                .disabled(!tile_enabled)
                .on_click({
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
        menu = menu.item(
            PopupMenuItem::new(
                rust_i18n::t!("menu.add_to_tiling", count = groups.len()).to_string(),
            )
            .icon(IconName::LayoutGrid)
            .on_click(move |_, window, cx| {
                app.update(cx, |this, cx| {
                    if let Some(group) = this.add_groups_to_tiling(set_id, groups.clone(), window) {
                        this.activate_group(group, window, cx);
                        this.tab_selection.clear();
                    } else {
                        this.error = Some(
                            rust_i18n::t!("tiling.too_many", max = MAX_TILING_SLOTS).to_string(),
                        );
                    }
                    cx.notify();
                });
            }),
        );
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
            PopupMenuItem::new(
                rust_i18n::t!("menu.tile_selected", count = selected_singles_count).to_string(),
            )
            .icon(IconName::LayoutGrid)
            .disabled(true),
        );
    }
    menu = menu.item(
        PopupMenuItem::new(rust_i18n::t!("menu.untile").to_string())
            .icon(IconName::Minimize2)
            .disabled(this_set_id.is_none())
            .on_click({
                let app = app.clone();
                move |_, _window, cx| {
                    if let Some(id) = this_set_id {
                        app.update(cx, |this, cx| this.untile_tiling(id, cx));
                    }
                }
            }),
    );
    menu.item(PopupMenuItem::separator())
        .item(
            PopupMenuItem::new(rust_i18n::t!("menu.close_tab").to_string())
                .icon(IconName::X)
                .on_click({
                    let app = app.clone();
                    move |_, window, cx| {
                        app.update(cx, |this, cx| {
                            this.close_group(group_id, window, cx);
                        });
                    }
                }),
        )
        .item(
            PopupMenuItem::new(rust_i18n::t!("menu.close_tiling_group").to_string())
                .icon(IconName::Trash)
                .disabled(this_set_id.is_none())
                .on_click({
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
        .item(
            PopupMenuItem::new(rust_i18n::t!("menu.close_other_tabs").to_string())
                .icon(IconName::Trash)
                .on_click({
                    let app = app.clone();
                    move |_, window, cx| {
                        app.update(cx, |this, cx| {
                            this.activate_tab(active_pane, window, cx);
                            this.close_other_tabs(active_pane, window, cx);
                        });
                    }
                }),
        )
}
