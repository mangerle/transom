//! 详情头部各分类专属动作按钮组。
//!
//! 设计原理（Why）：
//! - 将 Compose、Image、Network 等分类专用的按钮构建逻辑从主头部渲染剥离；
//! - 降低单文件代码复杂度，使单个文件生产代码行数维持在规范上限以内。

use super::*;
use crate::container::{ContainerWorkspace, ContainerWorkspaceEvent};
use gpui_kit::assets::IconName;
use transom_core::{
    ComposeAction, ComposeProject, ContainerImageSummary, ContainerLifecycleAction,
    ContainerNetworkSummary,
};

use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};

/// 编排项目「全部启动/全部停止」主行动按钮。
fn compose_lifecycle_button(
    view: &Entity<ContainerWorkspace>,
    names: Vec<String>,
    stop: bool,
) -> AnyElement {
    let view = view.clone();
    let label_key = if stop {
        "container.action.stop_all"
    } else {
        "container.action.start_all"
    };
    let button = Button::new(if stop {
        "compose-stop-all"
    } else {
        "compose-start-all"
    })
    .xsmall()
    .icon(if stop {
        IconName::Square
    } else {
        IconName::Play
    })
    .label(rust_i18n::t!(label_key).to_string())
    .on_click(move |_, _, cx| {
        let action = if stop {
            ContainerLifecycleAction::Stop
        } else {
            ContainerLifecycleAction::Start
        };
        view.update(cx, |workspace, cx| {
            for name in &names {
                workspace.container_lifecycle(name.clone(), action.clone(), cx);
            }
            cx.notify();
        });
    });
    if stop {
        button.ghost().into_any_element()
    } else {
        button.primary().into_any_element()
    }
}

/// 编排项目「更多」下拉里的命令项。
fn compose_menu_item(
    view: &Entity<ContainerWorkspace>,
    label_key: &'static str,
    icon: IconName,
    action: ComposeAction,
    name: String,
) -> PopupMenuItem {
    let view = view.clone();
    crate::widgets::menu_item(
        rust_i18n::t!(label_key).to_string(),
        icon,
        move |_, _, cx| {
            view.update(cx, |workspace, cx| {
                workspace.run_compose(name.clone(), action, cx);
            });
        },
    )
}

pub(super) fn compose_header_actions(
    view: &Entity<ContainerWorkspace>,
    project: &ComposeProject,
    all_containers: &[ContainerSummary],
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

    let mut actions = Vec::new();

    // 1. 主行动：一屏一个核心主行动（运行中显示「全部停止」，否则显示「全部启动」）
    actions.push(compose_lifecycle_button(view, names, any_running));

    // 2. 「更多」操作下拉菜单：收拢重新部署、重启、构建、拉取与删除
    let more_view = view.clone();
    let compose_name = project.name.clone();
    let more_btn = Button::new("compose-more-actions")
        .xsmall()
        .ghost()
        .icon(IconName::ChevronDown)
        .label(rust_i18n::t!("common.more").to_string())
        .dropdown_menu(move |menu, _, _| {
            let view = more_view.clone();
            let name = compose_name.clone();
            let mut menu = menu;
            menu = menu.item(compose_menu_item(
                &view,
                "container.action.compose_restart",
                IconName::RotateCw,
                ComposeAction::Restart,
                name.clone(),
            ));
            menu = menu.item(compose_menu_item(
                &view,
                "container.action.compose_up",
                IconName::RefreshCw,
                ComposeAction::Up,
                name.clone(),
            ));
            menu = menu.item(compose_menu_item(
                &view,
                "container.action.compose_build",
                IconName::Layers,
                ComposeAction::Build,
                name.clone(),
            ));
            menu = menu.item(compose_menu_item(
                &view,
                "container.action.compose_pull",
                IconName::Download,
                ComposeAction::Pull,
                name.clone(),
            ));
            menu = menu.item(PopupMenuItem::separator());
            menu.item(compose_menu_item(
                &view,
                "container.action.compose_down",
                IconName::Trash,
                ComposeAction::Down,
                name.clone(),
            ))
        })
        .into_any_element();

    actions.push(more_btn);
    actions
}

pub(super) fn image_header_actions(
    view: &Entity<ContainerWorkspace>,
    image: &ContainerImageSummary,
) -> Vec<AnyElement> {
    let image_id = image.id.clone();
    let push_ref = image.display_name();
    let tag_view = view.clone();
    let tag_id = image_id.clone();
    let push_view = view.clone();
    let push_id = image_id.clone();
    let push_reference = push_ref.clone();
    let save_view = view.clone();
    let save_id = image_id.clone();

    vec![
        Button::new("image-tag")
            .xsmall()
            .icon(IconName::TagPlus)
            .label(rust_i18n::t!("container.action.tag").to_string())
            .on_click(move |_, _, cx| {
                tag_view.update(cx, |_, cx| {
                    cx.emit(ContainerWorkspaceEvent::OpenImageTag(tag_id.clone()));
                });
            })
            .into_any_element(),
        Button::new("image-push")
            .xsmall()
            .icon(IconName::Upload)
            .label(rust_i18n::t!("container.action.push").to_string())
            .on_click(move |_, _, cx| {
                push_view.update(cx, |_, cx| {
                    cx.emit(ContainerWorkspaceEvent::OpenImagePush {
                        id: push_id.clone(),
                        reference: push_reference.clone(),
                    });
                });
            })
            .into_any_element(),
        Button::new("image-save")
            .xsmall()
            .icon(IconName::Download)
            .label(rust_i18n::t!("container.action.save_image").to_string())
            .on_click(move |_, _, cx| {
                save_view.update(cx, |_, cx| {
                    cx.emit(ContainerWorkspaceEvent::SaveImage(save_id.clone()));
                });
            })
            .into_any_element(),
        delete_button(
            view,
            "image-delete",
            ContainerConfirm::Image(image.id.clone()),
        ),
    ]
}

pub(super) fn network_header_actions(
    view: &Entity<ContainerWorkspace>,
    network: &ContainerNetworkSummary,
) -> Vec<AnyElement> {
    let network_id = network.id.clone();
    let connect_view = view.clone();
    vec![
        Button::new("network-connect")
            .xsmall()
            .icon(IconName::Link)
            .label(rust_i18n::t!("container.action.connect_network").to_string())
            .on_click(move |_, _, cx| {
                connect_view.update(cx, |_, cx| {
                    cx.emit(ContainerWorkspaceEvent::OpenNetworkConnect(
                        network_id.clone(),
                    ));
                });
            })
            .into_any_element(),
        delete_button(
            view,
            "network-delete",
            ContainerConfirm::Network(network.id.clone()),
        ),
    ]
}
