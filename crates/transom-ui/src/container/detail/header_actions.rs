//! 详情头部各分类专属动作按钮组。
//!
//! 设计原理（Why）：
//! - 将 Compose、Image、Network 等分类专用的按钮构建逻辑从主头部渲染剥离；
//! - 降低单文件代码复杂度，使单个文件生产代码行数维持在规范上限以内。

use super::*;
use gpui_kit::assets::IconName;
use transom_core::{
    ComposeAction, ComposeProject, ContainerCommand, ContainerImageSummary,
    ContainerNetworkSummary, ContainerRequestId, TabId,
};

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
    let start_names = names.clone();
    let stop_names = names;
    let compose_name = project.name.clone();
    let up_name = compose_name.clone();
    let restart_name = compose_name.clone();
    let build_name = compose_name.clone();
    let pull_name = compose_name.clone();
    let down_name = compose_name;

    vec![
        Button::new("compose-start-all")
            .xsmall()
            .icon(IconName::Play)
            .label(rust_i18n::t!("container.action.start_all").to_string())
            .on_click(cx.listener(move |this, _, _, cx| {
                for name in &start_names {
                    this.container_lifecycle(
                        tab_id,
                        name.clone(),
                        transom_core::ContainerLifecycleAction::Start,
                        cx,
                    );
                }
            }))
            .into_any_element(),
        Button::new("compose-stop-all")
            .xsmall()
            .icon(IconName::Square)
            .label(rust_i18n::t!("container.action.stop_all").to_string())
            .on_click(cx.listener(move |this, _, _, cx| {
                for name in &stop_names {
                    this.container_lifecycle(
                        tab_id,
                        name.clone(),
                        transom_core::ContainerLifecycleAction::Stop,
                        cx,
                    );
                }
            }))
            .into_any_element(),
        Button::new("compose-up")
            .xsmall()
            .icon(IconName::RefreshCw)
            .label(rust_i18n::t!("container.action.compose_up").to_string())
            .on_click(cx.listener(move |this, _, _, cx| {
                this.run_compose(tab_id, up_name.clone(), ComposeAction::Up, cx);
            }))
            .into_any_element(),
        Button::new("compose-restart")
            .xsmall()
            .icon(IconName::RotateCw)
            .label(rust_i18n::t!("container.action.compose_restart").to_string())
            .on_click(cx.listener(move |this, _, _, cx| {
                this.run_compose(tab_id, restart_name.clone(), ComposeAction::Restart, cx);
            }))
            .into_any_element(),
        Button::new("compose-build")
            .xsmall()
            .icon(IconName::Layers)
            .label(rust_i18n::t!("container.action.compose_build").to_string())
            .on_click(cx.listener(move |this, _, _, cx| {
                this.run_compose(tab_id, build_name.clone(), ComposeAction::Build, cx);
            }))
            .into_any_element(),
        Button::new("compose-pull")
            .xsmall()
            .icon(IconName::Download)
            .label(rust_i18n::t!("container.action.compose_pull").to_string())
            .on_click(cx.listener(move |this, _, _, cx| {
                this.run_compose(tab_id, pull_name.clone(), ComposeAction::Pull, cx);
            }))
            .into_any_element(),
        Button::new("compose-down")
            .xsmall()
            .icon(IconName::Trash)
            .label(rust_i18n::t!("container.action.compose_down").to_string())
            .on_click(cx.listener(move |this, _, _, cx| {
                this.run_compose(tab_id, down_name.clone(), ComposeAction::Down, cx);
            }))
            .into_any_element(),
    ]
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
                let push_ref = push_ref.clone();
                move |this, _, _, cx| {
                    let request_id = ContainerRequestId::new();
                    if let Some(workspace) = this.state.container_workspace_mut(tab_id) {
                        workspace.busy = true;
                    }
                    this.send_tracked(
                        tab_id,
                        request_id,
                        ContainerCommand::PushImage {
                            tab_id,
                            request_id,
                            image: push_ref.clone(),
                            tag: None,
                            auth: None,
                        },
                    );
                    cx.notify();
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
