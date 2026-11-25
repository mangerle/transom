//! HTTP、GraphQL、gRPC 与 WebSocket 的请求主体视图。

use super::ApiBodyMode;
use super::view::ApiWorkspaceParams;
use crate::palette::Palette;
use crate::theme::tokens::{rem_px, space, typography};
use crate::widgets::{IconButton, IconButtonSize, action_button, dropdown};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Disableable;
use gpui_kit::component::Sizable;
use gpui_kit::component::button::ButtonVariants;
use gpui_kit::component::input::{Editor, Input, Textarea};
use gpui_kit::component::radio::Radio;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_api::{ApiBodyType, ApiProtocol, ApiRawLanguage};
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
    let request = params.request;
    let body_type = request.map_or(ApiBodyType::Text, |request| request.body_type);
    let raw_language = request
        .map(|request| {
            request
                .body_type
                .raw_language()
                .unwrap_or(request.raw_language)
        })
        .unwrap_or_default();
    let mode = ApiBodyMode::from_body_type(body_type);
    let raw_view = params.view.clone();
    let controls = ApiBodyMode::ALL
        .into_iter()
        .fold(
            h_flex()
                .h(rems(crate::theme::tokens::size::LARGE_HEADER_HEIGHT))
                .w_full()
                .px(rems(space::SM))
                .gap(rems(space::MD))
                .items_center()
                .border_color(p.divider(p.border))
                .border_b_1(),
            |row, candidate| {
                let view = params.view.clone();
                row.child(
                    Radio::new(ElementId::Name(
                        format!("api-body-mode-{}", body_mode_id(candidate)).into(),
                    ))
                    .small()
                    .label(body_mode_label(candidate))
                    .checked(candidate == mode)
                    .on_click(move |_, window, cx| {
                        view.update(cx, |this, cx| this.set_body_mode(candidate, window, cx));
                    }),
                )
            },
        )
        .when(mode == ApiBodyMode::Raw, |row| {
            let format_view = params.view.clone();
            row.child(dropdown(
                "api-raw-language",
                raw_language,
                ApiRawLanguage::ALL
                    .into_iter()
                    .map(|language| (language, raw_language_label(language)))
                    .collect(),
                |this: &mut super::ApiWorkspace, language, window, cx| {
                    this.set_raw_language(language, window, cx);
                },
                raw_view,
            ))
            .child(div().flex_1())
            .child(
                action_button(
                    "api-body-format",
                    AppIcon::Ui(UiIcon::EDIT),
                    rust_i18n::t!("api.request.format_body").to_string(),
                )
                .ghost()
                .on_click(move |_, window, cx| {
                    format_view.update(cx, |this, cx| this.format_request_body(window, cx));
                }),
            )
        });

    let content = match body_type {
        ApiBodyType::None => {
            centered_body_message(rust_i18n::t!("api.request.no_body").to_string(), p)
        }
        ApiBodyType::Json
        | ApiBodyType::Text
        | ApiBodyType::Xml
        | ApiBodyType::JavaScript
        | ApiBodyType::Html => {
            render_code_body("api-request-body", &params.editors.request_body, p)
        }
        ApiBodyType::FormUrlEncoded | ApiBodyType::Multipart => {
            params.editors.form.clone().into_any_element()
        }
        ApiBodyType::Binary => render_binary_body(params, p),
    };
    v_flex()
        .size_full()
        .min_h_0()
        .child(controls)
        .child(content)
        .into_any_element()
}

fn body_mode_label(mode: ApiBodyMode) -> String {
    match mode {
        ApiBodyMode::None => rust_i18n::t!("api.request.body_none").to_string(),
        ApiBodyMode::FormData => rust_i18n::t!("api.request.body_multipart").to_string(),
        ApiBodyMode::UrlEncoded => rust_i18n::t!("api.request.body_form_urlencoded").to_string(),
        ApiBodyMode::Binary => rust_i18n::t!("api.request.body_binary").to_string(),
        ApiBodyMode::Raw => rust_i18n::t!("api.request.body_raw").to_string(),
    }
}

fn body_mode_id(mode: ApiBodyMode) -> &'static str {
    match mode {
        ApiBodyMode::None => "none",
        ApiBodyMode::FormData => "form-data",
        ApiBodyMode::UrlEncoded => "urlencoded",
        ApiBodyMode::Binary => "binary",
        ApiBodyMode::Raw => "raw",
    }
}

fn raw_language_label(language: ApiRawLanguage) -> String {
    match language {
        ApiRawLanguage::Json => "JSON".to_string(),
        ApiRawLanguage::Xml => "XML".to_string(),
        ApiRawLanguage::JavaScript => "JavaScript".to_string(),
        ApiRawLanguage::Text => "Text".to_string(),
        ApiRawLanguage::Html => "HTML".to_string(),
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
        .child(render_code_body(
            "api-graphql-body-editor",
            &params.editors.request_body,
            p,
        ))
        .child(field_label(
            rust_i18n::t!("api.request.graphql_variables").to_string(),
            p,
        ))
        .child(
            div()
                .h(rem_px(8.0, params.rem))
                .min_h_0()
                .child(Textarea::new(&params.editors.graphql_variables).size_full()),
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
                .child(Input::new(&params.editors.grpc_service).flex_1().min_w_0())
                .child(Input::new(&params.editors.grpc_method).w(rems(12.0))),
        )
        .child(field_label(
            rust_i18n::t!("api.request.grpc_message").to_string(),
            p,
        ))
        .child(render_code_body(
            "api-graphql-body-editor",
            &params.editors.request_body,
            p,
        ))
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
                    AppIconElement::new(AppIcon::Ui(if params.exec.ws_connected {
                        UiIcon::CHECK
                    } else {
                        UiIcon::WARNING
                    }))
                    .xsmall(),
                )
                .child(
                    div()
                        .text_color(if params.exec.ws_connected {
                            p.success
                        } else {
                            p.muted_foreground
                        })
                        .child(if params.exec.ws_connected {
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
                    .disabled(!params.exec.ws_connected)
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
                    .disabled(!params.exec.ws_connected)
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
                .child(Textarea::new(&params.editors.websocket_input).size_full()),
        )
        .into_any_element()
}

fn render_code_body(
    id: &'static str,
    state: &Entity<gpui_kit::component::input::EditorState>,
    p: &Palette,
) -> AnyElement {
    div()
        .id(id)
        .flex_1()
        .min_h_0()
        .w_full()
        .p(rems(space::SM))
        .child(
            div()
                .size_full()
                .rounded(p.radius.md)
                .border_1()
                .border_color(p.divider(p.border))
                .overflow_hidden()
                .child(
                    Editor::new(state)
                        .appearance(false)
                        .bordered(false)
                        .size_full(),
                ),
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
