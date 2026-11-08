//! 详情头部各分类专属动作按钮组。
//!
//! 设计原理（Why）：
//! - 将 Compose、Image、Network 等分类专用的按钮构建逻辑从主头部渲染剥离；
//! - 降低单文件代码复杂度，使单个文件生产代码行数维持在规范上限以内。

use super::*;
use gpui_kit::assets::IconName;
use transom_core::{
    ComposeAction, ComposeProject, ContainerImageSummary, ContainerNetworkSummary, TabId,
};

use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};

pub(super) fn compose_header_actions(
    tab_id: TabId,
    project: &ComposeProject,
    all_containers: &[ContainerSummary],
    cx: &mut Context<AppShell>,
) -> Vec<AnyElement> {
    let names: Vec<String> = all_containers
        .iter()
        .filter(|item| item.compose_project.as_deref() == Some(project.name.as_str()))
        .map(|item| item.name.clone())
        .collect();
    let any_running = all_containers.iter().any(|item| {
        item.compose_project.as_deref() == Some(project.name.as_str())
            && item.state == transom_core::ContainerState::Running
    });

    let start_names = names.clone();
    let stop_names = names;
    let compose_name = project.name.clone();
    let up_name = compose_name.clone();
    let restart_name = compose_name.clone();
    let build_name = compose_name.clone();
    let pull_name = compose_name.clone();
    let down_name = compose_name;
    let app = cx.entity().clone();

    let mut actions = Vec::new();

    // 1. 主行动：一屏一个核心主行动（运行中显示「全部停止」，否则显示「全部启动」）
    if any_running {
        actions.push(
            Button::new("compose-stop-all")
                .xsmall()
                .ghost()
                .icon(IconName::Square)
                .label(rust_i18n::t!("container.action.stop_all").to_string())
                .on_click(cx.listener(move |this, _, _, cx| {
                    for name in &stop_names {
                        if let Some(workspace) = this.state.container_workspace(tab_id) {
                            workspace.update(cx, |workspace, cx| {
                                workspace.container_lifecycle(
                                    name.clone(),
                                    transom_core::ContainerLifecycleAction::Stop,
                                    cx,
                                );
                            });
                        }
                    }
                }))
                .into_any_element(),
        );
    } else {
        actions.push(
            Button::new("compose-start-all")
                .xsmall()
                .primary()
                .icon(IconName::Play)
                .label(rust_i18n::t!("container.action.start_all").to_string())
                .on_click(cx.listener(move |this, _, _, cx| {
                    for name in &start_names {
                        if let Some(workspace) = this.state.container_workspace(tab_id) {
                            workspace.update(cx, |workspace, cx| {
                                workspace.container_lifecycle(
                                    name.clone(),
                                    transom_core::ContainerLifecycleAction::Start,
                                    cx,
                                );
                            });
                        }
                    }
                }))
                .into_any_element(),
        );
    }

    // 2. 「更多」操作下拉菜单：收拢重新部署、重启、构建、拉取与删除
    let more_btn = Button::new("compose-more-actions")
        .xsmall()
        .ghost()
        .icon(IconName::ChevronDown)
        .label(rust_i18n::t!("common.more").to_string())
        .dropdown_menu({
            let app = app.clone();
            move |menu, _, _| {
                let mut menu = menu;
                let restart_app = app.clone();
                let restart_name = restart_name.clone();
                menu = menu.item(crate::widgets::menu_item(
                    rust_i18n::t!("container.action.compose_restart").to_string(),
                    IconName::RotateCw,
                    move |_, _, cx| {
                        restart_app.update(cx, |this, cx| {
                            this.run_compose(
                                tab_id,
                                restart_name.clone(),
                                ComposeAction::Restart,
                                cx,
                            );
                        });
                    },
                ));

                let up_app = app.clone();
                let up_name = up_name.clone();
                menu = menu.item(crate::widgets::menu_item(
                    rust_i18n::t!("container.action.compose_up").to_string(),
                    IconName::RefreshCw,
                    move |_, _, cx| {
                        up_app.update(cx, |this, cx| {
                            this.run_compose(tab_id, up_name.clone(), ComposeAction::Up, cx);
                        });
                    },
                ));

                let build_app = app.clone();
                let build_name = build_name.clone();
                menu = menu.item(crate::widgets::menu_item(
                    rust_i18n::t!("container.action.compose_build").to_string(),
                    IconName::Layers,
                    move |_, _, cx| {
                        build_app.update(cx, |this, cx| {
                            this.run_compose(tab_id, build_name.clone(), ComposeAction::Build, cx);
                        });
                    },
                ));

                let pull_app = app.clone();
                let pull_name = pull_name.clone();
                menu = menu.item(crate::widgets::menu_item(
                    rust_i18n::t!("container.action.compose_pull").to_string(),
                    IconName::Download,
                    move |_, _, cx| {
                        pull_app.update(cx, |this, cx| {
                            this.run_compose(tab_id, pull_name.clone(), ComposeAction::Pull, cx);
                        });
                    },
                ));

                menu = menu.item(PopupMenuItem::separator());

                let down_app = app.clone();
                let down_name = down_name.clone();
                menu = menu.item(crate::widgets::menu_item(
                    rust_i18n::t!("container.action.compose_down").to_string(),
                    IconName::Trash,
                    move |_, _, cx| {
                        down_app.update(cx, |this, cx| {
                            this.run_compose(tab_id, down_name.clone(), ComposeAction::Down, cx);
                        });
                    },
                ));

                menu
            }
        })
        .into_any_element();

    actions.push(more_btn);
    actions
}

pub(super) fn image_header_actions(
    tab_id: TabId,
    image: &ContainerImageSummary,
    cx: &mut Context<AppShell>,
) -> Vec<AnyElement> {
    let image_id = image.id.clone();
    let push_ref = image.display_name();

    vec![
        Button::new("image-tag")
            .xsmall()
            .icon(IconName::TagPlus)
            .label(rust_i18n::t!("container.action.tag").to_string())
            .on_click(cx.listener({
                let image_id = image_id.clone();
                move |this, _, window, cx| {
                    this.open_image_tag(tab_id, image_id.clone(), window, cx);
                }
            }))
            .into_any_element(),
        Button::new("image-push")
            .xsmall()
            .icon(IconName::Upload)
            .label(rust_i18n::t!("container.action.push").to_string())
            .on_click(cx.listener({
                let image_id = image_id.clone();
                let push_ref = push_ref.clone();
                move |this, _, window, cx| {
                    this.open_container_push_dialog(
                        tab_id,
                        image_id.clone(),
                        push_ref.clone(),
                        window,
                        cx,
                    );
                }
            }))
            .into_any_element(),
        Button::new("image-save")
            .xsmall()
            .icon(IconName::Download)
            .label(rust_i18n::t!("container.action.save_image").to_string())
            .on_click(cx.listener({
                let image_id = image_id.clone();
                move |this, _, _, cx| {
                    this.save_image(tab_id, image_id.clone(), cx);
                }
            }))
            .into_any_element(),
        delete_button(
            "image-delete",
            cx,
            tab_id,
            ContainerConfirm::Image(image.id.clone()),
        ),
    ]
}

pub(super) fn network_header_actions(
    tab_id: TabId,
    network: &ContainerNetworkSummary,
    cx: &mut Context<AppShell>,
) -> Vec<AnyElement> {
    let network_id = network.id.clone();
    vec![
        Button::new("network-connect")
            .xsmall()
            .icon(IconName::Link)
            .label(rust_i18n::t!("container.action.connect_network").to_string())
            .on_click(cx.listener({
                let network_id = network_id.clone();
                move |this, _, window, cx| {
                    this.open_network_connect(tab_id, network_id.clone(), window, cx);
                }
            }))
            .into_any_element(),
        delete_button(
            "network-delete",
            cx,
            tab_id,
            ContainerConfirm::Network(network.id.clone()),
        ),
    ]
}
