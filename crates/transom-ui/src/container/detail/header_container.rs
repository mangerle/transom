//! 容器详情头部的专属动作构建：主行动（停止/恢复/启动）、次行动（终端跳转）与「更多」管理菜单。
//!
//! 设计原理（Why）：
//! - 将容器专用的操作按钮从 `header.rs` 剥离，避免单文件行数超出 500 行规范；
//! - 遵循「一屏一个主行动」原则，运行中仅展示「停止」与「终端」，非运行展示「启动」，
//!   重启、更新、导出、提交、克隆与删除等全部收纳进「更多」下拉菜单，并在删除前加分隔线。

use super::*;
use gpui_kit::assets::IconName;
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use transom_core::{ContainerSummary, TabId};

/// 构建容器详情头部右侧的精简操作组。
pub(super) fn container_header_actions(
    tab_id: TabId,
    container: &ContainerSummary,
    detail_tab: ContainerDetailTab,
    overview_raw: bool,
    cx: &mut Context<AppShell>,
) -> Vec<AnyElement> {
    let mut actions = Vec::new();
    let app = cx.entity().clone();
    let name = container.name.clone();
    let id = container.id.clone();

    let running = container.state == transom_core::ContainerState::Running;
    let paused = container.state == transom_core::ContainerState::Paused;

    if paused {
        actions.push(
            Button::new("container-unpause")
                .xsmall()
                .primary()
                .icon(IconName::Play)
                .label(rust_i18n::t!("container.action.unpause").to_string())
                .on_click(cx.listener({
                    let name = name.clone();
                    move |this, _, _, cx| {
                        this.container_lifecycle(
                            tab_id,
                            name.clone(),
                            transom_core::ContainerLifecycleAction::Unpause,
                            cx,
                        );
                    }
                }))
                .into_any_element(),
        );
        actions.push(
            Button::new("container-stop")
                .xsmall()
                .ghost()
                .icon(IconName::Square)
                .label(rust_i18n::t!("container.action.stop").to_string())
                .on_click(cx.listener({
                    let name = name.clone();
                    move |this, _, _, cx| {
                        this.container_lifecycle(
                            tab_id,
                            name.clone(),
                            transom_core::ContainerLifecycleAction::Stop,
                            cx,
                        );
                    }
                }))
                .into_any_element(),
        );
    } else if running {
        actions.push(
            Button::new("container-stop")
                .xsmall()
                .ghost()
                .icon(IconName::Square)
                .label(rust_i18n::t!("container.action.stop").to_string())
                .on_click(cx.listener({
                    let name = name.clone();
                    move |this, _, _, cx| {
                        this.container_lifecycle(
                            tab_id,
                            name.clone(),
                            transom_core::ContainerLifecycleAction::Stop,
                            cx,
                        );
                    }
                }))
                .into_any_element(),
        );
        if detail_tab != ContainerDetailTab::Terminal {
            actions.push(
                Button::new("container-jump-terminal")
                    .xsmall()
                    .ghost()
                    .icon(IconName::Terminal)
                    .label(rust_i18n::t!("container.detail.terminal").to_string())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.set_container_detail_tab(
                            tab_id,
                            ContainerDetailTab::Terminal,
                            window,
                            cx,
                        );
                    }))
                    .into_any_element(),
            );
        }
    } else {
        actions.push(
            Button::new("container-start")
                .xsmall()
                .primary()
                .icon(IconName::Play)
                .label(rust_i18n::t!("container.action.start").to_string())
                .on_click(cx.listener({
                    let name = name.clone();
                    move |this, _, _, cx| {
                        this.container_lifecycle(
                            tab_id,
                            name.clone(),
                            transom_core::ContainerLifecycleAction::Start,
                            cx,
                        );
                    }
                }))
                .into_any_element(),
        );
    }

    let more_btn = Button::new("container-detail-more")
        .xsmall()
        .ghost()
        .icon(IconName::Ellipsis)
        .tooltip(rust_i18n::t!("common.more").to_string())
        .dropdown_menu({
            let app = app.clone();
            let name = name.clone();
            let id = id.clone();
            move |menu, _, _| {
                let mut menu = menu;

                if running {
                    let app_p = app.clone();
                    let name_p = name.clone();
                    menu = menu.item(
                        PopupMenuItem::new(rust_i18n::t!("container.action.pause").to_string())
                            .on_click(move |_, _, cx| {
                                app_p.update(cx, |this, cx| {
                                    this.container_lifecycle(
                                        tab_id,
                                        name_p.clone(),
                                        transom_core::ContainerLifecycleAction::Pause,
                                        cx,
                                    );
                                });
                            }),
                    );
                }

                if running || paused {
                    let app_r = app.clone();
                    let name_r = name.clone();
                    menu = menu.item(
                        PopupMenuItem::new(rust_i18n::t!("container.action.restart").to_string())
                            .on_click(move |_, _, cx| {
                                app_r.update(cx, |this, cx| {
                                    this.container_lifecycle(
                                        tab_id,
                                        name_r.clone(),
                                        transom_core::ContainerLifecycleAction::Restart,
                                        cx,
                                    );
                                });
                            }),
                    );
                    let app_k = app.clone();
                    let name_k = name.clone();
                    menu = menu.item(
                        PopupMenuItem::new(rust_i18n::t!("container.action.kill").to_string())
                            .on_click(move |_, _, cx| {
                                app_k.update(cx, |this, cx| {
                                    this.container_lifecycle(
                                        tab_id,
                                        name_k.clone(),
                                        transom_core::ContainerLifecycleAction::Kill { signal: None },
                                        cx,
                                    );
                                });
                            }),
                    );
                    menu = menu.item(PopupMenuItem::separator());
                }

                let app_u = app.clone();
                let name_u = name.clone();
                menu = menu.item(
                    PopupMenuItem::new(rust_i18n::t!("container.action.update").to_string())
                        .on_click(move |_, window, cx| {
                            app_u.update(cx, |this, cx| {
                                this.open_container_update(tab_id, name_u.clone(), window, cx);
                            });
                        }),
                );

                let app_e = app.clone();
                let name_e = name.clone();
                menu = menu.item(
                    PopupMenuItem::new(rust_i18n::t!("container.action.export_config").to_string())
                        .on_click(move |_, _, cx| {
                            app_e.update(cx, |this, cx| {
                                this.open_container_export(tab_id, name_e.clone(), cx);
                            });
                        }),
                );

                let app_c = app.clone();
                let name_c = name.clone();
                menu = menu.item(
                    PopupMenuItem::new(rust_i18n::t!("container.action.commit").to_string())
                        .on_click(move |_, window, cx| {
                            app_c.update(cx, |this, cx| {
                                this.open_container_commit(tab_id, name_c.clone(), window, cx);
                            });
                        }),
                );

                let app_cl = app.clone();
                let id_cl = id.clone();
                let name_cl = name.clone();
                menu = menu.item(
                    PopupMenuItem::new(rust_i18n::t!("container.action.clone").to_string())
                        .on_click(move |_, window, cx| {
                            app_cl.update(cx, |this, cx| {
                                this.clone_container(tab_id, &id_cl, &name_cl, window, cx);
                            });
                        }),
                );

                menu = menu.item(PopupMenuItem::separator());
                let app_d = app.clone();
                let name_d = name.clone();
                menu = menu.item(
                    PopupMenuItem::new(rust_i18n::t!("common.delete").to_string())
                        .on_click(move |_, _, cx| {
                            app_d.update(cx, |this, cx| {
                                if let Some(workspace) = this.state.container_workspace_mut(tab_id) {
                                    workspace.confirm = Some(ContainerConfirm::Container(name_d.clone()));
                                }
                                cx.notify();
                            });
                        }),
                );

                menu
            }
        });

    actions.push(more_btn.into_any_element());
    actions
}
