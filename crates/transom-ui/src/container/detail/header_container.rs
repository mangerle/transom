//! 容器详情头部的专属动作构建：主行动（停止/恢复/启动）、次行动（终端跳转）与「更多」管理菜单。
//!
//! 设计原理（Why）：
//! - 将容器专用的操作按钮从 `header.rs` 剥离，避免单文件行数超出 500 行规范；
//! - 遵循「一屏一个主行动」原则，运行中仅展示「停止」与「终端」，非运行展示「启动」，
//!   重启、更新、导出、提交、克隆与删除等全部收纳进「更多」下拉菜单，并在删除前加分隔线；
//! - 渲染只持有工作区实体句柄：生命周期、删除确认等实体动作直接 `update` 实体，
//!   更新、导出、提交、克隆、重建与终端跳转需要外壳表单或窗口，统一发事件上报。

use super::*;
use crate::container::{ContainerWorkspace, ContainerWorkspaceEvent};
use crate::widgets::{IconButton, action_button};
use gpui_kit::component::menu::{DropdownMenu, PopupMenu, PopupMenuItem};
use transom_core::{ContainerLifecycleAction, ContainerSummary};
use transom_ui_icons::DomainIcon;
use transom_ui_icons::UiIcon;

/// 把生命周期动作接到工作区实体：按钮样式在调用点声明，回调只投递命令。
fn lifecycle_button(
    view: &Entity<ContainerWorkspace>,
    name: &str,
    action: ContainerLifecycleAction,
    button: Button,
) -> AnyElement {
    let view = view.clone();
    let name = name.to_string();
    button
        .on_click(move |_, _, cx| {
            view.update(cx, |workspace, cx| {
                workspace.container_lifecycle(name.clone(), action.clone(), cx);
            });
        })
        .into_any_element()
}

/// 构建容器详情头部右侧的精简操作组。
pub(super) fn container_header_actions(
    view: &Entity<ContainerWorkspace>,
    container: &ContainerSummary,
    detail_tab: ContainerDetailTab,
) -> Vec<AnyElement> {
    let name = container.name.clone();
    let id = container.id.clone();
    let running = container.state == transom_core::ContainerState::Running;
    let paused = container.state == transom_core::ContainerState::Paused;
    let mut actions = lifecycle_actions(view, &name, running, paused, detail_tab);
    actions.push(container_more_button(view, &name, &id, running, paused));
    actions
}

/// 生命周期主行动：暂停态显示恢复与停止，运行态显示停止与终端跳转，其余显示启动。
fn lifecycle_actions(
    view: &Entity<ContainerWorkspace>,
    name: &str,
    running: bool,
    paused: bool,
    detail_tab: ContainerDetailTab,
) -> Vec<AnyElement> {
    let mut actions = Vec::new();
    if paused {
        actions.push(lifecycle_button(
            view,
            name,
            ContainerLifecycleAction::Unpause,
            action_button(
                "container-unpause",
                UiIcon::RUN,
                rust_i18n::t!("container.action.unpause").to_string(),
            )
            .primary(),
        ));
        actions.push(stop_action(view, name));
    } else if running {
        actions.push(stop_action(view, name));
        if detail_tab != ContainerDetailTab::Terminal {
            actions.push(terminal_jump_button(view));
        }
    } else {
        actions.push(lifecycle_button(
            view,
            name,
            ContainerLifecycleAction::Start,
            action_button(
                "container-start",
                UiIcon::RUN,
                rust_i18n::t!("container.action.start").to_string(),
            )
            .primary(),
        ));
    }
    actions
}

fn stop_action(view: &Entity<ContainerWorkspace>, name: &str) -> AnyElement {
    lifecycle_button(
        view,
        name,
        ContainerLifecycleAction::Stop,
        action_button(
            "container-stop",
            UiIcon::STOP,
            rust_i18n::t!("container.action.stop").to_string(),
        )
        .ghost(),
    )
}

fn terminal_jump_button(view: &Entity<ContainerWorkspace>) -> AnyElement {
    let terminal_view = view.clone();
    action_button(
        "container-jump-terminal",
        DomainIcon::TERMINAL,
        rust_i18n::t!("container.detail.terminal").to_string(),
    )
    .ghost()
    .on_click(move |_, _, cx| {
        terminal_view.update(cx, |_, cx| {
            cx.emit(ContainerWorkspaceEvent::SetDetailTab(
                ContainerDetailTab::Terminal,
            ));
        });
    })
    .into_any_element()
}

/// 「更多」管理菜单：生命周期补项 + 更新/导出/提交/克隆/重建 + 删除。
fn container_more_button(
    view: &Entity<ContainerWorkspace>,
    name: &str,
    id: &str,
    running: bool,
    paused: bool,
) -> AnyElement {
    let view = view.clone();
    let name = name.to_string();
    let id = id.to_string();
    IconButton::new(
        "container-detail-more",
        UiIcon::MORE_HORIZONTAL,
        rust_i18n::t!("common.more").to_string(),
    )
    .render()
    .dropdown_menu(move |mut menu, _, _| {
        if running {
            menu = menu_pause(menu, &view, &name);
        }
        if running || paused {
            let lifecycle_menu = menu_restart(menu, &view, &name);
            let lifecycle_menu = menu_kill(lifecycle_menu, &view, &name);
            menu = lifecycle_menu.item(PopupMenuItem::separator());
        }
        let manage_menu = menu_update(menu, &view, &name);
        let manage_menu = menu_export(manage_menu, &view, &name);
        let manage_menu = menu_commit(manage_menu, &view, &name);
        let manage_menu = menu_clone(manage_menu, &view, &name, &id);
        let manage_menu = menu_recreate(manage_menu, &view, &name, &id);
        let manage_menu = manage_menu.item(PopupMenuItem::separator());
        menu_delete(manage_menu, &view, &name)
    })
    .into_any_element()
}

fn menu_pause(menu: PopupMenu, view: &Entity<ContainerWorkspace>, name: &str) -> PopupMenu {
    let action_view = view.clone();
    let action_name = name.to_string();
    menu.item(
        PopupMenuItem::new(rust_i18n::t!("container.action.pause").to_string()).on_click(
            move |_, _, cx| {
                action_view.update(cx, |workspace, cx| {
                    workspace.container_lifecycle(
                        action_name.clone(),
                        ContainerLifecycleAction::Pause,
                        cx,
                    );
                });
            },
        ),
    )
}

fn menu_restart(menu: PopupMenu, view: &Entity<ContainerWorkspace>, name: &str) -> PopupMenu {
    let restart_view = view.clone();
    let restart_name = name.to_string();
    menu.item(
        PopupMenuItem::new(rust_i18n::t!("container.action.restart").to_string()).on_click(
            move |_, _, cx| {
                restart_view.update(cx, |workspace, cx| {
                    workspace.container_lifecycle(
                        restart_name.clone(),
                        ContainerLifecycleAction::Restart,
                        cx,
                    );
                });
            },
        ),
    )
}

fn menu_kill(menu: PopupMenu, view: &Entity<ContainerWorkspace>, name: &str) -> PopupMenu {
    let kill_view = view.clone();
    let kill_name = name.to_string();
    menu.item(
        PopupMenuItem::new(rust_i18n::t!("container.action.kill").to_string()).on_click(
            move |_, _, cx| {
                kill_view.update(cx, |workspace, cx| {
                    workspace.container_lifecycle(
                        kill_name.clone(),
                        ContainerLifecycleAction::Kill { signal: None },
                        cx,
                    );
                });
            },
        ),
    )
}

fn menu_update(menu: PopupMenu, view: &Entity<ContainerWorkspace>, name: &str) -> PopupMenu {
    let update_view = view.clone();
    let update_name = name.to_string();
    menu.item(
        PopupMenuItem::new(rust_i18n::t!("container.action.update").to_string()).on_click(
            move |_, _, cx| {
                update_view.update(cx, |_, cx| {
                    cx.emit(ContainerWorkspaceEvent::OpenContainerUpdate(
                        update_name.clone(),
                    ));
                });
            },
        ),
    )
}

fn menu_export(menu: PopupMenu, view: &Entity<ContainerWorkspace>, name: &str) -> PopupMenu {
    let export_view = view.clone();
    let export_name = name.to_string();
    menu.item(
        PopupMenuItem::new(rust_i18n::t!("container.action.export_config").to_string()).on_click(
            move |_, _, cx| {
                export_view.update(cx, |_, cx| {
                    cx.emit(ContainerWorkspaceEvent::OpenContainerExport(
                        export_name.clone(),
                    ));
                });
            },
        ),
    )
}

fn menu_commit(menu: PopupMenu, view: &Entity<ContainerWorkspace>, name: &str) -> PopupMenu {
    let commit_view = view.clone();
    let commit_name = name.to_string();
    menu.item(
        PopupMenuItem::new(rust_i18n::t!("container.action.commit").to_string()).on_click(
            move |_, _, cx| {
                commit_view.update(cx, |_, cx| {
                    cx.emit(ContainerWorkspaceEvent::OpenContainerCommit(
                        commit_name.clone(),
                    ));
                });
            },
        ),
    )
}

fn menu_clone(
    menu: PopupMenu,
    view: &Entity<ContainerWorkspace>,
    name: &str,
    id: &str,
) -> PopupMenu {
    let clone_view = view.clone();
    let clone_id = id.to_string();
    let clone_name = name.to_string();
    menu.item(
        PopupMenuItem::new(rust_i18n::t!("container.action.clone").to_string()).on_click(
            move |_, _, cx| {
                clone_view.update(cx, |_, cx| {
                    cx.emit(ContainerWorkspaceEvent::CloneContainer {
                        id: clone_id.clone(),
                        name: clone_name.clone(),
                    });
                });
            },
        ),
    )
}

fn menu_recreate(
    menu: PopupMenu,
    view: &Entity<ContainerWorkspace>,
    name: &str,
    id: &str,
) -> PopupMenu {
    let recreate_view = view.clone();
    let recreate_id = id.to_string();
    let recreate_name = name.to_string();
    menu.item(
        PopupMenuItem::new(rust_i18n::t!("container.detail.recreate_menu").to_string()).on_click(
            move |_, _, cx| {
                recreate_view.update(cx, |_, cx| {
                    cx.emit(ContainerWorkspaceEvent::RecreateContainer {
                        id: recreate_id.clone(),
                        name: recreate_name.clone(),
                    });
                });
            },
        ),
    )
}

fn menu_delete(menu: PopupMenu, view: &Entity<ContainerWorkspace>, name: &str) -> PopupMenu {
    let delete_view = view.clone();
    let delete_name = name.to_string();
    menu.item(
        PopupMenuItem::new(rust_i18n::t!("common.delete").to_string()).on_click(move |_, _, cx| {
            delete_view.update(cx, |workspace, cx| {
                workspace.state.modals.confirm =
                    Some(ContainerConfirm::Container(delete_name.clone()));
                cx.notify();
            });
        }),
    )
}
