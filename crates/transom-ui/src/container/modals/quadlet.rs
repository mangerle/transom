//! Podman Quadlet 单元列表模态弹窗。

use crate::container::{ContainerWorkspace, ContainerWorkspaceState};
use crate::palette::Palette;
use crate::widgets::{action_button, dialog_actions, dialog_body, dialog_cancel_button};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::*;
use transom_core::ContainerQuadletAction;
use transom_ui_icons::AppIcon;
use transom_ui_icons::AppIconElement;
use transom_ui_icons::UiIcon;

/// 渲染宿主 Quadlet 单元列表与 systemd 操作。
///
/// 设计原理（Why）：单元数据、加载态与在途请求都归工作区实体，渲染只读快照；
/// 操作与刷新回调直接 `update` 实体，不再经外壳转发。
pub(crate) fn render_container_quadlet_dialog_body(
    view: &Entity<ContainerWorkspace>,
    state: &ContainerWorkspaceState,
    p: &Palette,
) -> AnyElement {
    let loading = state.quadlets_loading;
    let error = state.quadlets_error.clone();

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
                        .child(rust_i18n::t!("container.quadlet.loading").to_string()),
                ),
        );
    }
    if let Some(error) = error {
        content = content.child(div().text_sm().text_color(p.danger).child(error));
    }
    if !loading && state.quadlets.is_empty() && state.quadlets_error.is_none() {
        content = content.child(
            div()
                .text_sm()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("container.quadlet.empty").to_string()),
        );
    }
    for unit in &state.quadlets {
        let unit_name = unit.name.clone();
        let active = unit.active == Some(true);
        let state_key = if active {
            "container.quadlet.active"
        } else {
            "container.quadlet.inactive"
        };
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
                    .child(div().text_sm().child(unit.name.clone()))
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child(format!(
                                "{} · {} · {}",
                                unit.kind,
                                rust_i18n::t!(state_key),
                                unit.path
                            )),
                    ),
            )
            .child(quadlet_action_button(
                view,
                &unit_name,
                ContainerQuadletAction::Start,
                "container.action.start",
                active,
            ))
            .child(quadlet_action_button(
                view,
                &unit_name,
                ContainerQuadletAction::Stop,
                "container.action.stop",
                !active,
            ))
            .child(quadlet_action_button(
                view,
                &unit_name,
                ContainerQuadletAction::Restart,
                "container.action.restart",
                false,
            ));
        content = content.child(row);
    }

    let refresh = action_button(
        "container-quadlet-refresh",
        UiIcon::REFRESH,
        rust_i18n::t!("common.refresh").to_string(),
    )
    .ghost()
    .on_click({
        let view = view.clone();
        move |_, _, cx| {
            view.update(cx, |workspace, cx| workspace.refresh_quadlets(cx));
        }
    });
    let close = dialog_cancel_button(
        "container-quadlet-close",
        rust_i18n::t!("common.close").to_string(),
        {
            let view = view.clone();
            move |_, _, cx| {
                view.update(cx, |workspace, cx| workspace.close_quadlets_dialog(cx));
            }
        },
    );
    let footer = dialog_actions([refresh.into_any_element(), close.into_any_element()]);
    dialog_body(
        "container-quadlet-modal",
        rust_i18n::t!("container.quadlet.title").to_string(),
        Some(rust_i18n::t!("container.quadlet.subtitle").to_string()),
        content,
        Some(footer),
    )
}

/// 生成单个 Quadlet 操作按钮；`disabled` 为真时按钮灰显但仍占位。
fn quadlet_action_button(
    view: &Entity<ContainerWorkspace>,
    unit_name: &str,
    action: ContainerQuadletAction,
    label_key: &'static str,
    disabled: bool,
) -> AnyElement {
    let id = match action {
        ContainerQuadletAction::Start => format!("quadlet-start-{unit_name}"),
        ContainerQuadletAction::Stop => format!("quadlet-stop-{unit_name}"),
        ContainerQuadletAction::Restart => format!("quadlet-restart-{unit_name}"),
    };
    let unit = unit_name.to_string();
    Button::new(ElementId::Name(id.into()))
        .xsmall()
        .ghost()
        .label(rust_i18n::t!(label_key).to_string())
        .disabled(disabled)
        .on_click({
            let view = view.clone();
            move |_, _, cx| {
                view.update(cx, |workspace, cx| {
                    workspace.quadlet_action(unit.clone(), action, cx);
                });
            }
        })
        .into_any_element()
}
