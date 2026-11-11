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
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use transom_core::{ContainerLifecycleAction, ContainerSummary};
use transom_ui_icons::AppIcon;
use transom_ui_icons::AppIconElement;
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
    let mut actions = Vec::new();
    let name = container.name.clone();
    let id = container.id.clone();

    let running = container.state == transom_core::ContainerState::Running;
    let paused = container.state == transom_core::ContainerState::Paused;

    if paused {
        actions.push(lifecycle_button(
            view,
            &name,
            ContainerLifecycleAction::Unpause,
            Button::new("container-unpause")
                .xsmall()
                .primary()
                .child(AppIconElement::new(AppIcon::Ui(UiIcon::RUN)).small())
                .label(rust_i18n::t!("container.action.unpause").to_string()),
        ));
        actions.push(lifecycle_button(
            view,
            &name,
            ContainerLifecycleAction::Stop,
            Button::new("container-stop")
                .xsmall()
                .ghost()
                .child(AppIconElement::new(AppIcon::Ui(UiIcon::STOP)).small())
                .label(rust_i18n::t!("container.action.stop").to_string()),
        ));
    } else if running {
        actions.push(lifecycle_button(
            view,
            &name,
            ContainerLifecycleAction::Stop,
            Button::new("container-stop")
                .xsmall()
                .ghost()
                .child(AppIconElement::new(AppIcon::Ui(UiIcon::STOP)).small())
                .label(rust_i18n::t!("container.action.stop").to_string()),
        ));
        if detail_tab != ContainerDetailTab::Terminal {
            let terminal_view = view.clone();
            actions.push(
                Button::new("container-jump-terminal")
                    .xsmall()
                    .ghost()
                    .child(AppIconElement::new(AppIcon::Domain(DomainIcon::TERMINAL)).small())
                    .label(rust_i18n::t!("container.detail.terminal").to_string())
                    .on_click(move |_, _, cx| {
                        terminal_view.update(cx, |_, cx| {
                            cx.emit(ContainerWorkspaceEvent::SetDetailTab(
                                ContainerDetailTab::Terminal,
                            ));
                        });
                    })
                    .into_any_element(),
            );
        }
    } else {
        actions.push(lifecycle_button(
            view,
            &name,
            ContainerLifecycleAction::Start,
            Button::new("container-start")
                .xsmall()
                .primary()
                .child(AppIconElement::new(AppIcon::Ui(UiIcon::RUN)).small())
                .label(rust_i18n::t!("container.action.start").to_string()),
        ));
    }

    let more_btn = Button::new("container-detail-more")
        .xsmall()
        .ghost()
        .child(AppIconElement::new(AppIcon::Ui(UiIcon::MORE_HORIZONTAL)).small())
        .accessibility_label(rust_i18n::t!("common.more").to_string())
        .tooltip(rust_i18n::t!("common.more").to_string())
        .dropdown_menu({
            let view = view.clone();
            let name = name.clone();
            let id = id.clone();
            move |menu, _, _| {
                let mut menu = menu;

                if running {
                    let action_view = view.clone();
                    let action_name = name.clone();
                    menu = menu.item(
                        PopupMenuItem::new(rust_i18n::t!("container.action.pause").to_string())
                            .on_click(move |_, _, cx| {
                                action_view.update(cx, |workspace, cx| {
                                    workspace.container_lifecycle(
                                        action_name.clone(),
                                        ContainerLifecycleAction::Pause,
                                        cx,
                                    );
                                });
                            }),
                    );
                }

                if running || paused {
                    let restart_view = view.clone();
                    let restart_name = name.clone();
                    menu = menu.item(
                        PopupMenuItem::new(rust_i18n::t!("container.action.restart").to_string())
                            .on_click(move |_, _, cx| {
                                restart_view.update(cx, |workspace, cx| {
                                    workspace.container_lifecycle(
                                        restart_name.clone(),
                                        ContainerLifecycleAction::Restart,
                                        cx,
                                    );
                                });
                            }),
                    );
                    let kill_view = view.clone();
                    let kill_name = name.clone();
                    menu = menu.item(
                        PopupMenuItem::new(rust_i18n::t!("container.action.kill").to_string())
                            .on_click(move |_, _, cx| {
                                kill_view.update(cx, |workspace, cx| {
                                    workspace.container_lifecycle(
                                        kill_name.clone(),
                                        ContainerLifecycleAction::Kill { signal: None },
                                        cx,
                                    );
                                });
                            }),
                    );
                    menu = menu.item(PopupMenuItem::separator());
                }

                let update_view = view.clone();
                let update_name = name.clone();
                menu = menu.item(
                    PopupMenuItem::new(rust_i18n::t!("container.action.update").to_string())
                        .on_click(move |_, _, cx| {
                            update_view.update(cx, |_, cx| {
                                cx.emit(ContainerWorkspaceEvent::OpenContainerUpdate(
                                    update_name.clone(),
                                ));
                            });
                        }),
                );

                let export_view = view.clone();
                let export_name = name.clone();
                menu = menu.item(
                    PopupMenuItem::new(rust_i18n::t!("container.action.export_config").to_string())
                        .on_click(move |_, _, cx| {
                            export_view.update(cx, |_, cx| {
                                cx.emit(ContainerWorkspaceEvent::OpenContainerExport(
                                    export_name.clone(),
                                ));
                            });
                        }),
                );

                let commit_view = view.clone();
                let commit_name = name.clone();
                menu = menu.item(
                    PopupMenuItem::new(rust_i18n::t!("container.action.commit").to_string())
                        .on_click(move |_, _, cx| {
                            commit_view.update(cx, |_, cx| {
                                cx.emit(ContainerWorkspaceEvent::OpenContainerCommit(
                                    commit_name.clone(),
                                ));
                            });
                        }),
                );

                let clone_view = view.clone();
                let clone_id = id.clone();
                let clone_name = name.clone();
                menu = menu.item(
                    PopupMenuItem::new(rust_i18n::t!("container.action.clone").to_string())
                        .on_click(move |_, _, cx| {
                            clone_view.update(cx, |_, cx| {
                                cx.emit(ContainerWorkspaceEvent::CloneContainer {
                                    id: clone_id.clone(),
                                    name: clone_name.clone(),
                                });
                            });
                        }),
                );

                let recreate_view = view.clone();
                let recreate_id = id.clone();
                let recreate_name = name.clone();
                menu = menu.item(
                    PopupMenuItem::new(rust_i18n::t!("container.detail.recreate_menu").to_string())
                        .on_click(move |_, _, cx| {
                            recreate_view.update(cx, |_, cx| {
                                cx.emit(ContainerWorkspaceEvent::RecreateContainer {
                                    id: recreate_id.clone(),
                                    name: recreate_name.clone(),
                                });
                            });
                        }),
                );

                menu = menu.item(PopupMenuItem::separator());
                let delete_view = view.clone();
                let delete_name = name.clone();
                menu = menu.item(
                    PopupMenuItem::new(rust_i18n::t!("common.delete").to_string()).on_click(
                        move |_, _, cx| {
                            delete_view.update(cx, |workspace, cx| {
                                workspace.state.confirm =
                                    Some(ContainerConfirm::Container(delete_name.clone()));
                                cx.notify();
                            });
                        },
                    ),
                );

                menu
            }
        });

    actions.push(more_btn.into_any_element());
    actions
}
