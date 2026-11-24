//! HTTP、GraphQL、gRPC 与 WebSocket 的请求主体视图。

use super::workspace_request_view::render_text_panel;
use super::workspace_view::ApiWorkspaceParams;
use crate::palette::Palette;
use crate::theme::tokens::{rem_px, space, typography};
use crate::widgets::{IconButton, IconButtonSize, action_button, dropdown};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Disableable;
use gpui_kit::component::button::ButtonVariants;
use gpui_kit::component::input::{Input, Textarea};
use gpui_kit::*;
use transom_api::{ApiBodyType, ApiProtocol};
use transom_ui_icons::{AppIcon, AppIconElement, UiIcon};

pub(super) fn render_body(params: &ApiWorkspaceParams<'_>, p: &Palette) -> AnyElement {
    match params.protocol {
        ApiProtocol::Http => render_http_body(params, p),
        ApiProtocol::GraphQl => render_graphql_body(params, p),
        ApiProtocol::Grpc => render_grpc_body(params, p),
        ApiProtocol::WebSocket => render_websocket_body(params, p),
    }
}

fn render_http_body(params: &ApiWorkspaceParams<'_>, p: &Palette) -> AnyElement {
    let body_type = params
        .request
        .map_or(ApiBodyType::Text, |request| request.body_type);
    let view = params.view.clone();
    let body_type_control = dropdown(
        "api-request-body-type",
        body_type,
        ApiBodyType::ALL
            .into_iter()
            .map(|kind| (kind, body_type_label(kind)))
            .collect(),
        |this: &mut super::ApiWorkspace, kind, window, cx| {
            this.set_body_type(kind, window, cx);
        },
        view,
    );
    let content = match body_type {
        ApiBodyType::None => {
            centered_body_message(rust_i18n::t!("api.request.no_body").to_string(), p)
        }
        ApiBodyType::Json | ApiBodyType::Text | ApiBodyType::Xml => {
            render_text_panel("api-request-body", params.request_body.clone(), p)
        }
        ApiBodyType::FormUrlEncoded | ApiBodyType::Multipart => {
            params.form_editor.clone().into_any_element()
        }
        ApiBodyType::Binary => render_binary_body(params, p),
    };
    v_flex()
        .size_full()
        .min_h_0()
        .child(
            h_flex()
                .h(rems(crate::theme::tokens::size::PANE_HEADER_HEIGHT))
                .px(rems(space::SM))
                .gap(rems(space::SM))
                .items_center()
                .child(
                    div()
                        .text_size(rems(typography::CAPTION))
                        .text_color(p.muted_foreground)
                        .child(rust_i18n::t!("api.request.body_type").to_string()),
                )
                .child(body_type_control),
        )
        .child(content)
        .into_any_element()
}

fn body_type_label(body_type: ApiBodyType) -> String {
    match body_type {
        ApiBodyType::None => rust_i18n::t!("api.request.body_none").to_string(),
        ApiBodyType::Json => "JSON".to_string(),
        ApiBodyType::Text => rust_i18n::t!("api.request.body_text").to_string(),
        ApiBodyType::Xml => "XML".to_string(),
        ApiBodyType::FormUrlEncoded => {
            rust_i18n::t!("api.request.body_form_urlencoded").to_string()
        }
        ApiBodyType::Multipart => rust_i18n::t!("api.request.body_multipart").to_string(),
        ApiBodyType::Binary => rust_i18n::t!("api.request.body_binary").to_string(),
    }
}

fn render_binary_body(params: &ApiWorkspaceParams<'_>, p: &Palette) -> AnyElement {
    let view = params.view.clone();
    let path = params
        .request
        .map(|request| request.body_file.as_str())
        .unwrap_or_default();
    let label = if path.is_empty() {
        rust_i18n::t!("api.request.no_file_selected").to_string()
    } else {
        path.to_string()
    };
    v_flex()
        .id("api-binary-body")
        .flex_1()
        .min_h_0()
        .w_full()
        .p(rems(space::SM))
        .items_start()
        .gap(rems(space::XS))
        .child(
            action_button(
                "api-binary-file",
                AppIcon::Ui(UiIcon::ADD_FILE),
                rust_i18n::t!("api.request.choose_file").to_string(),
            )
            .on_click(move |_, window, cx| {
                view.update(cx, |this, cx| this.pick_binary_file(window, cx));
            }),
        )
        .child(
            div()
                .text_size(rems(typography::CAPTION))
                .text_color(if path.is_empty() {
                    p.muted_foreground
                } else {
                    p.foreground
                })
                .child(label),
        )
        .into_any_element()
}

fn centered_body_message(message: String, p: &Palette) -> AnyElement {
    v_flex()
        .flex_1()
        .min_h_0()
        .items_center()
        .justify_center()
        .text_color(p.muted_foreground)
        .child(message)
        .into_any_element()
}

fn render_graphql_body(params: &ApiWorkspaceParams<'_>, p: &Palette) -> AnyElement {
    v_flex()
        .id("api-graphql-body")
        .flex_1()
        .min_h_0()
        .w_full()
        .p(rems(space::SM))
        .gap(rems(space::XS))
        .child(field_label(
            rust_i18n::t!("api.request.graphql_query").to_string(),
            p,
        ))
        .child(
            div()
                .flex_1()
                .min_h_0()
                .child(Textarea::new(params.request_body).size_full()),
        )
        .child(field_label(
            rust_i18n::t!("api.request.graphql_variables").to_string(),
            p,
        ))
        .child(
            div()
                .h(rem_px(8.0, params.rem))
                .min_h_0()
                .child(Textarea::new(params.graphql_variables).size_full()),
        )
        .into_any_element()
}

fn render_grpc_body(params: &ApiWorkspaceParams<'_>, p: &Palette) -> AnyElement {
    v_flex()
        .id("api-grpc-body")
        .flex_1()
        .min_h_0()
        .w_full()
        .p(rems(space::SM))
        .gap(rems(space::XS))
        .child(
            h_flex()
                .gap(rems(space::SM))
                .items_center()
                .child(
                    div()
                        .w(rems(7.0))
                        .text_color(p.muted_foreground)
                        .text_size(rems(typography::CAPTION))
                        .child(rust_i18n::t!("api.request.grpc_service").to_string()),
                )
                .child(Input::new(params.grpc_service).flex_1().min_w_0())
                .child(Input::new(params.grpc_method).w(rems(12.0))),
        )
        .child(field_label(
            rust_i18n::t!("api.request.grpc_message").to_string(),
            p,
        ))
        .child(
            div()
                .flex_1()
                .min_h_0()
                .child(Textarea::new(params.request_body).size_full()),
        )
        .into_any_element()
}

fn render_websocket_body(params: &ApiWorkspaceParams<'_>, p: &Palette) -> AnyElement {
    let connect_view = params.view.clone();
    let send_view = params.view.clone();
    let close_view = params.view.clone();
    v_flex()
        .id("api-websocket-body")
        .flex_1()
        .min_h_0()
        .w_full()
        .p(rems(space::SM))
        .gap(rems(space::XS))
        .child(
            h_flex()
                .gap(rems(space::XS))
                .items_center()
                .child(
                    AppIconElement::new(AppIcon::Ui(if params.ws_connected {
                        UiIcon::CHECK
                    } else {
                        UiIcon::WARNING
                    }))
                    .xsmall(),
                )
                .child(
                    div()
                        .text_color(if params.ws_connected {
                            p.success
                        } else {
                            p.muted_foreground
                        })
                        .child(if params.ws_connected {
                            rust_i18n::t!("api.websocket.connected").to_string()
                        } else {
                            rust_i18n::t!("api.websocket.not_connected").to_string()
                        }),
                )
                .child(div().flex_1())
                .child(
                    action_button(
                        "api-ws-connect",
                        AppIcon::Ui(UiIcon::FETCH),
                        rust_i18n::t!("api.request.websocket_connect").to_string(),
                    )
                    .on_click(move |_, window, cx| {
                        connect_view.update(cx, |this, cx| this.connect_websocket(window, cx));
                    }),
                )
                .child(
                    action_button(
                        "api-ws-send",
                        AppIcon::Ui(UiIcon::RUN),
                        rust_i18n::t!("api.request.websocket_send").to_string(),
                    )
                    .primary()
                    .disabled(!params.ws_connected)
                    .on_click(move |_, window, cx| {
                        send_view.update(cx, |this, cx| this.send_websocket_text(window, cx));
                    }),
                )
                .child(
                    IconButton::new(
                        "api-ws-close",
                        AppIcon::Ui(UiIcon::CLOSE),
                        rust_i18n::t!("api.request.websocket_close").to_string(),
                    )
                    .size(IconButtonSize::Compact)
                    .render()
                    .disabled(!params.ws_connected)
                    .on_click(move |_, _, cx| {
                        close_view.update(cx, |this, cx| this.close_websocket(cx));
                    }),
                ),
        )
        .child(field_label(
            rust_i18n::t!("api.request.websocket_input").to_string(),
            p,
        ))
        .child(
            div()
                .flex_1()
                .min_h_0()
                .child(Textarea::new(params.websocket_input).size_full()),
        )
        .into_any_element()
}

fn field_label(label: String, p: &Palette) -> AnyElement {
    div()
        .pb(rems(space::XXS))
        .text_color(p.muted_foreground)
        .text_size(rems(typography::CAPTION))
        .child(label)
        .into_any_element()
}
