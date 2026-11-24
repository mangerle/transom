//! HTTP、GraphQL、gRPC 与 WebSocket 的请求主体视图。

use super::workspace_request_view::render_text_panel;
use super::workspace_view::ApiWorkspaceParams;
use crate::palette::Palette;
use crate::theme::tokens::{rem_px, space, typography};
use crate::widgets::{IconButton, IconButtonSize, action_button};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Disableable;
use gpui_kit::component::button::ButtonVariants;
use gpui_kit::component::input::{Input, Textarea};
use gpui_kit::*;
use transom_api::ApiProtocol;
use transom_ui_icons::{AppIcon, AppIconElement, UiIcon};

pub(super) fn render_body(params: &ApiWorkspaceParams<'_>, p: &Palette) -> AnyElement {
    match params.protocol {
        ApiProtocol::Http => render_text_panel("api-request-body", params.request_body.clone(), p),
        ApiProtocol::GraphQl => render_graphql_body(params, p),
        ApiProtocol::Grpc => render_grpc_body(params, p),
        ApiProtocol::WebSocket => render_websocket_body(params, p),
    }
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
