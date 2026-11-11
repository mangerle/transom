//! Podman Pod 列表模态弹窗。

use crate::container::{ContainerWorkspace, ContainerWorkspaceState};
use crate::palette::Palette;
use crate::widgets::{dialog_actions, dialog_body, dialog_cancel_button};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::*;
use transom_core::ContainerPodAction;
use transom_ui_icons::AppIcon;
use transom_ui_icons::AppIconElement;
use transom_ui_icons::UiIcon;

/// 渲染 Pod 列表弹窗：列表、加载/错误状态与启停按钮。
///
/// 设计原理（Why）：Pod 数据、加载态与在途请求都归工作区实体，渲染只读快照；
/// 启停与刷新回调直接 `update` 实体，不再经外壳转发。
pub(crate) fn render_container_pods_dialog_body(
    view: &Entity<ContainerWorkspace>,
    state: &ContainerWorkspaceState,
    p: &Palette,
) -> AnyElement {
    let loading = state.pods_loading;
    let error = state.pods_error.clone();

    let mut content = v_flex().gap_2().w_full();
    if loading {
        content = content.child(
            h_flex()
                .gap_2()
                .items_center()
                .child(AppIconElement::new(AppIcon::Ui(UiIcon::REFRESH)).xsmall())
                .child(
                    div()
                        .text_sm()
                        .child(rust_i18n::t!("container.pods.loading").to_string()),
                ),
        );
    }
    if let Some(error) = error {
        content = content.child(div().text_sm().text_color(p.danger).child(error));
    }
    if !loading && state.pods.is_empty() && state.pods_error.is_none() {
        content = content.child(
            div()
                .text_sm()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("container.pods.empty").to_string()),
        );
    }
    for pod in &state.pods {
        let pod_id = pod.id.clone();
        let status_running = pod.status.to_ascii_lowercase().contains("running");
        let row = h_flex()
            .gap_2()
            .items_center()
            .p_2()
            .rounded(p.radius.md)
            .bg(p.muted.opacity(0.25))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(div().text_sm().child(pod.name.clone()))
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child(format!(
                                "{} · {} {}",
                                pod.status,
                                pod.containers,
                                rust_i18n::t!("container.pods.containers")
                            )),
                    ),
            )
            .child(pod_action_button(
                view,
                &pod_id,
                ContainerPodAction::Start,
                "container.action.start",
                status_running,
            ))
            .child(pod_action_button(
                view,
                &pod_id,
                ContainerPodAction::Stop,
                "container.action.stop",
                !status_running,
            ))
            .child(pod_action_button(
                view,
                &pod_id,
                ContainerPodAction::Restart,
                "container.action.restart",
                false,
            ));
        content = content.child(row);
    }

    let refresh = Button::new("container-pods-refresh")
        .ghost()
        .xsmall()
        .child(AppIconElement::new(AppIcon::Ui(UiIcon::REFRESH)).small())
        .label(rust_i18n::t!("common.refresh").to_string())
        .on_click({
            let view = view.clone();
            move |_, _, cx| {
                view.update(cx, |workspace, cx| workspace.refresh_pods(cx));
            }
        });
    let close = dialog_cancel_button(
        "container-pods-close",
        rust_i18n::t!("common.close").to_string(),
        {
            let view = view.clone();
            move |_, _, cx| {
                view.update(cx, |workspace, cx| workspace.close_pods_dialog(cx));
            }
        },
    );
    let footer = dialog_actions([refresh.into_any_element(), close.into_any_element()]);
    dialog_body(
        "container-pods-modal",
        rust_i18n::t!("container.pods.title").to_string(),
        Some(rust_i18n::t!("container.pods.subtitle").to_string()),
        content,
        Some(footer),
    )
}

/// 生成单个 Pod 操作按钮；`disabled` 为真时按钮灰显但仍占位。
fn pod_action_button(
    view: &Entity<ContainerWorkspace>,
    pod_id: &str,
    action: ContainerPodAction,
    label_key: &'static str,
    disabled: bool,
) -> AnyElement {
    let id = match action {
        ContainerPodAction::Start => format!("pod-start-{pod_id}"),
        ContainerPodAction::Stop => format!("pod-stop-{pod_id}"),
        ContainerPodAction::Restart => format!("pod-restart-{pod_id}"),
        _ => format!("pod-action-{pod_id}"),
    };
    let pod = pod_id.to_string();
    Button::new(ElementId::Name(id.into()))
        .xsmall()
        .ghost()
        .label(rust_i18n::t!(label_key).to_string())
        .disabled(disabled)
        .on_click({
            let view = view.clone();
            move |_, _, cx| {
                view.update(cx, |workspace, cx| {
                    workspace.pod_action(pod.clone(), action, cx);
                });
            }
        })
        .into_any_element()
}
