//! API 工作区的协议选择、高级配置、Mock、历史和代码生成视图。

use super::workspace_view::ApiWorkspaceParams;
use super::{ApiAuxiliarySection, ApiWorkspace};
use crate::palette::Palette;
use crate::theme::tokens::space;
use crate::widgets::{IconButton, IconButtonSize, action_button, dropdown};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Disableable;
use gpui_kit::component::input::{Textarea, TextareaState};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_api::{ApiEnvironmentId, ApiProtocol, CodeTarget};
use transom_ui_icons::{AppIcon, UiIcon};

/// 协议、环境和辅助面板工具条。
pub(crate) fn render_protocol_bar(params: &ApiWorkspaceParams<'_>, p: &Palette) -> AnyElement {
    let view = params.view.clone();
    let protocol_options = [
        (ApiProtocol::Http, "HTTP".to_string()),
        (ApiProtocol::GraphQl, "GraphQL".to_string()),
        (
            ApiProtocol::Grpc,
            rust_i18n::t!("api.protocol.grpc_raw").to_string(),
        ),
        (ApiProtocol::WebSocket, "WebSocket".to_string()),
    ];
    let protocol = dropdown(
        "api-request-protocol",
        params.protocol,
        protocol_options.to_vec(),
        |this: &mut ApiWorkspace, protocol, _, cx| this.set_protocol(protocol, cx),
        view.clone(),
    );
    let environment_options = params
        .project
        .environments
        .iter()
        .map(|environment| (Some(environment.id), environment.name.clone()))
        .collect::<Vec<_>>();
    let environment = dropdown(
        "api-request-environment",
        params.environment_id,
        environment_options,
        |this: &mut ApiWorkspace, environment_id: Option<ApiEnvironmentId>, window, cx| {
            if let Some(environment_id) = environment_id {
                this.set_environment(environment_id, window, cx);
            }
        },
        view.clone(),
    );
    let view_for_ws = view.clone();
    let view_for_send_ws = view.clone();
    let view_for_close_ws = view.clone();
    let ws_controls = if params.protocol == ApiProtocol::WebSocket {
        h_flex()
            .gap(rems(space::XS))
            .items_center()
            .child(
                action_button(
                    "api-ws-connect",
                    AppIcon::Ui(UiIcon::FETCH),
                    if params.ws_connected {
                        rust_i18n::t!("api.request.websocket_connected").to_string()
                    } else {
                        rust_i18n::t!("api.request.websocket_connect").to_string()
                    },
                )
                .on_click(move |_, window, cx| {
                    view_for_ws.update(cx, |this, cx| this.connect_websocket(window, cx));
                }),
            )
            .child(
                action_button(
                    "api-ws-send",
                    AppIcon::Ui(UiIcon::RUN),
                    rust_i18n::t!("api.request.websocket_send").to_string(),
                )
                .disabled(!params.ws_connected)
                .on_click(move |_, window, cx| {
                    view_for_send_ws.update(cx, |this, cx| this.send_websocket_text(window, cx));
                }),
            )
            .child(
                action_button(
                    "api-ws-close",
                    AppIcon::Ui(UiIcon::CLOSE),
                    rust_i18n::t!("api.request.websocket_close").to_string(),
                )
                .disabled(!params.ws_connected)
                .on_click(move |_, _, cx| {
                    view_for_close_ws.update(cx, |this, cx| this.close_websocket(cx));
                }),
            )
            .child(
                Textarea::new(params.websocket_input)
                    .flex_1()
                    .min_w_0()
                    .h(rems(2.0)),
            )
            .into_any_element()
    } else {
        div().into_any_element()
    };

    h_flex()
        .flex_wrap()
        .gap(rems(space::XS))
        .items_center()
        .child(protocol)
        .child(environment)
        .child(div().flex_1().min_w_0().child(ws_controls))
        .child(render_auxiliary_tabs(params, p))
        .child(render_codegen_button(params, p))
        .into_any_element()
}

fn render_auxiliary_tabs(params: &ApiWorkspaceParams<'_>, _p: &Palette) -> AnyElement {
    let view = params.view.clone();
    h_flex()
        .gap(rems(space::XS))
        .items_center()
        .children([
            auxiliary_button(
                "api-aux-environment",
                "api.request.aux_environment",
                ApiAuxiliarySection::Environment,
                params.auxiliary_section,
                view.clone(),
            ),
            auxiliary_button(
                "api-aux-cookies",
                "api.request.aux_cookies",
                ApiAuxiliarySection::Cookies,
                params.auxiliary_section,
                view.clone(),
            ),
            auxiliary_button(
                "api-aux-auth",
                "api.request.aux_auth",
                ApiAuxiliarySection::Auth,
                params.auxiliary_section,
                view.clone(),
            ),
            auxiliary_button(
                "api-aux-assertions",
                "api.request.aux_assertions",
                ApiAuxiliarySection::Assertions,
                params.auxiliary_section,
                view.clone(),
            ),
            auxiliary_button(
                "api-aux-mock",
                "api.request.aux_mock",
                ApiAuxiliarySection::Mock,
                params.auxiliary_section,
                view.clone(),
            ),
            auxiliary_button(
                "api-aux-history",
                "api.request.aux_history",
                ApiAuxiliarySection::History,
                params.auxiliary_section,
                view,
            ),
        ])
        .into_any_element()
}

fn auxiliary_button(
    id: &'static str,
    label_key: &'static str,
    section: ApiAuxiliarySection,
    current: Option<ApiAuxiliarySection>,
    view: Entity<ApiWorkspace>,
) -> AnyElement {
    IconButton::new(
        id,
        AppIcon::Ui(UiIcon::SETTINGS),
        rust_i18n::t!(label_key).to_string(),
    )
    .size(IconButtonSize::Compact)
    .selected(current == Some(section))
    .render()
    .on_click(move |_, _, cx| {
        view.update(cx, |this, cx| this.toggle_auxiliary(section, cx));
    })
    .into_any_element()
}

fn render_codegen_button(params: &ApiWorkspaceParams<'_>, _p: &Palette) -> AnyElement {
    let view = params.view.clone();
    let options = [
        (CodeTarget::Curl, "cURL".to_string()),
        (CodeTarget::JavaScriptFetch, "JavaScript fetch".to_string()),
        (CodeTarget::PythonRequests, "Python requests".to_string()),
        (CodeTarget::RustReqwest, "Rust reqwest".to_string()),
    ];
    let current = CodeTarget::Curl;
    dropdown(
        "api-codegen",
        current,
        options.to_vec(),
        |this: &mut ApiWorkspace, target, _, cx| this.copy_code(target, cx),
        view,
    )
}

/// 高级配置面板；只渲染当前选中的页签，避免四类编辑区同时占满高度。
pub(crate) fn render_auxiliary_panel(params: &ApiWorkspaceParams<'_>, p: &Palette) -> AnyElement {
    let Some(section) = params.auxiliary_section else {
        return div().into_any_element();
    };
    let body = match section {
        ApiAuxiliarySection::Environment => editor("api-env-editor", params.environment_editor, p),
        ApiAuxiliarySection::Cookies => editor("api-cookie-editor", params.cookies_editor, p),
        ApiAuxiliarySection::Auth => editor("api-auth-editor", params.auth, p),
        ApiAuxiliarySection::Assertions => editor("api-assertions-editor", params.assertions, p),
        ApiAuxiliarySection::Mock => {
            let view = params.view.clone();
            v_flex()
                .gap(rems(space::XS))
                .child(editor("api-mock-editor", params.mock_editor, p))
                .child(
                    h_flex()
                        .gap(rems(space::XS))
                        .child(
                            action_button(
                                "api-mock-start",
                                AppIcon::Ui(UiIcon::RUN),
                                rust_i18n::t!("api.request.mock_start").to_string(),
                            )
                            .on_click({
                                let view = view.clone();
                                move |_, window, cx| {
                                    view.update(cx, |this, cx| this.start_mock(window, cx));
                                }
                            }),
                        )
                        .child(
                            action_button(
                                "api-mock-stop",
                                AppIcon::Ui(UiIcon::STOP),
                                rust_i18n::t!("api.request.mock_stop").to_string(),
                            )
                            .disabled(params.mock_port.is_none())
                            .on_click(move |_, _, cx| {
                                view.update(cx, |this, cx| this.stop_mock(cx));
                            }),
                        )
                        .when_some(params.mock_port, |row, port| {
                            row.child(
                                div()
                                    .text_color(p.muted_foreground)
                                    .child(format!("http://127.0.0.1:{port}")),
                            )
                        }),
                )
                .into_any_element()
        }
        ApiAuxiliarySection::History => editor("api-history-view", params.history_editor, p),
    };
    v_flex()
        .flex_1()
        .min_h_0()
        .w_full()
        .mx(rems(space::SM))
        .mb(rems(space::SM))
        .p(rems(space::SM))
        .rounded(p.radius.md)
        .bg(p.surface.control_selected)
        .child(body)
        .into_any_element()
}

fn editor(id: &'static str, state: &Entity<TextareaState>, _p: &Palette) -> AnyElement {
    v_flex()
        .id(id)
        .size_full()
        .min_h_0()
        .child(
            Textarea::new(state)
                .readonly(id == "api-history-view")
                .size_full(),
        )
        .into_any_element()
}
