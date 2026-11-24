//! API 请求编辑器：请求工具条、协议输入和请求级页签。

use super::workspace_advanced_view::{render_auxiliary_panel, render_workspace_toolbar};
use super::workspace_view::ApiWorkspaceParams;
use super::{ApiRequestTab, ApiWorkspace};
use crate::palette::Palette;
use crate::theme::tokens::{rem_px, size, space, typography};
use crate::widgets::{IconButton, IconButtonSize, action_button, dropdown};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Disableable;
use gpui_kit::component::Sizable;
use gpui_kit::component::button::ButtonVariants;
use gpui_kit::component::input::{Input, Textarea};
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::*;
use transom_api::{ApiAuth, ApiMethod, ApiProtocol};
use transom_ui_icons::{AppIcon, AppIconElement, FileTypeIcon, UiIcon};

const REQUEST_TABS: [ApiRequestTab; 4] = [
    ApiRequestTab::Body,
    ApiRequestTab::Headers,
    ApiRequestTab::Auth,
    ApiRequestTab::Assertions,
];

pub(super) fn render_request_editor(params: &ApiWorkspaceParams<'_>, p: &Palette) -> AnyElement {
    let view = params.view;
    let method_options = ApiMethod::ALL
        .into_iter()
        .map(|method| (method, method.as_str().to_string()))
        .collect();
    let method = dropdown(
        "api-request-method",
        params.method,
        method_options,
        |this: &mut ApiWorkspace, method, _, cx| this.set_method(method, cx),
        view.clone(),
    );
    let view_for_save = view.clone();
    let view_for_send = view.clone();
    let view_for_delete = view.clone();
    let send_icon = if params.protocol == ApiProtocol::WebSocket {
        AppIcon::Ui(UiIcon::FETCH)
    } else {
        AppIcon::Ui(UiIcon::RUN)
    };

    let mut content = v_flex()
        .size_full()
        .min_h_0()
        .overflow_hidden()
        .bg(p.background)
        .child(render_workspace_toolbar(params, p))
        .child(
            h_flex()
                .h(rems(size::LARGE_HEADER_HEIGHT))
                .px(rems(space::SM))
                .gap(rems(space::XS))
                .items_center()
                .border_color(p.divider(p.border))
                .border_b_1()
                .child(method)
                .child(Input::new(params.request_url).flex_1().min_w_0())
                .child(
                    action_button(
                        "api-request-save",
                        AppIcon::Ui(UiIcon::SAVE),
                        rust_i18n::t!("api.request.save").to_string(),
                    )
                    .on_click(move |_, _, cx| {
                        view_for_save.update(cx, |this, cx| this.save_project(cx));
                    }),
                )
                .child(
                    action_button(
                        "api-request-send",
                        send_icon,
                        if params.protocol == ApiProtocol::WebSocket {
                            rust_i18n::t!("api.request.websocket_connect").to_string()
                        } else {
                            rust_i18n::t!("api.request.send").to_string()
                        },
                    )
                    .primary()
                    .disabled(params.selected_request.is_none() || params.sending)
                    .on_click(move |_, window, cx| {
                        view_for_send.update(cx, |this, cx| this.send_request(window, cx));
                    }),
                )
                .child(
                    IconButton::new(
                        "api-request-delete",
                        AppIcon::Ui(UiIcon::DELETE),
                        rust_i18n::t!("api.request.delete").to_string(),
                    )
                    .size(IconButtonSize::Compact)
                    .render()
                    .disabled(params.selected_request.is_none())
                    .on_click(move |_, window, cx| {
                        view_for_delete.update(cx, |this, cx| this.delete_selected(window, cx));
                    }),
                ),
        )
        .child(
            h_flex()
                .px(rems(space::SM))
                .py(rems(space::XS))
                .gap(rems(space::SM))
                .items_center()
                .child(
                    div()
                        .w(rems(7.0))
                        .text_color(p.muted_foreground)
                        .text_size(rems(typography::CAPTION))
                        .child(rust_i18n::t!("api.request.name").to_string()),
                )
                .child(Input::new(params.request_name).flex_1().min_w_0()),
        );

    if let Some(error) = params.error {
        content = content.child(
            div()
                .mx(rems(space::SM))
                .mt(rems(space::SM))
                .p(rems(space::SM))
                .rounded(p.radius.md)
                .bg(p.surface.control_selected)
                .text_color(p.danger)
                .child(error.to_string()),
        );
    }
    if let Some(notice) = params.notice {
        content = content.child(
            div()
                .mx(rems(space::SM))
                .mt(rems(space::SM))
                .p(rems(space::SM))
                .rounded(p.radius.md)
                .bg(p.surface.control_selected)
                .text_color(p.success)
                .child(notice.to_string()),
        );
    }

    let editor = if params.auxiliary_section.is_some() {
        render_auxiliary_panel(params, p)
    } else if params.selected_request.is_some() {
        render_tabbed_request(params, p)
    } else {
        render_empty_request(p)
    };
    content.child(editor).into_any_element()
}

fn render_tabbed_request(params: &ApiWorkspaceParams<'_>, p: &Palette) -> AnyElement {
    let view = params.view.clone();
    let selected = REQUEST_TABS
        .iter()
        .position(|tab| *tab == params.request_tab)
        .unwrap_or(0);
    let tabs = REQUEST_TABS.into_iter().map(|tab| request_tab(tab, params));
    let bar = TabBar::new("api-request-tabs")
        .underline()
        .small()
        .w_full()
        .selected_index(selected)
        .children(tabs)
        .on_click(move |index, _, cx| {
            if let Some(tab) = REQUEST_TABS.get(*index).copied() {
                view.update(cx, |this, cx| this.set_request_tab(tab, cx));
            }
        });
    let content = match params.request_tab {
        ApiRequestTab::Body => render_body(params, p),
        ApiRequestTab::Headers => {
            render_text_panel("api-request-headers", params.request_headers.clone(), p)
        }
        ApiRequestTab::Auth => render_text_panel("api-request-auth", params.auth.clone(), p),
        ApiRequestTab::Assertions => {
            render_text_panel("api-request-assertions", params.assertions.clone(), p)
        }
    };
    v_flex()
        .flex_1()
        .min_h_0()
        .w_full()
        .child(
            div()
                .px(rems(space::SM))
                .border_color(p.divider(p.border))
                .border_b_1()
                .child(bar),
        )
        .child(content)
        .into_any_element()
}

fn request_tab(tab: ApiRequestTab, params: &ApiWorkspaceParams<'_>) -> Tab {
    let request = params.request;
    let (label, icon) = match tab {
        ApiRequestTab::Body => (
            match params.protocol {
                ApiProtocol::Http => rust_i18n::t!("api.request.body").to_string(),
                ApiProtocol::GraphQl => rust_i18n::t!("api.request.graphql_query").to_string(),
                ApiProtocol::Grpc => rust_i18n::t!("api.request.grpc_message").to_string(),
                ApiProtocol::WebSocket => {
                    rust_i18n::t!("api.request.websocket_messages").to_string()
                }
            },
            match params.protocol {
                ApiProtocol::Http => AppIcon::Ui(UiIcon::EDIT),
                ApiProtocol::GraphQl => AppIcon::FileType(FileTypeIcon::GRAPHQL),
                ApiProtocol::Grpc => AppIcon::Ui(UiIcon::DEPENDENCIES),
                ApiProtocol::WebSocket => AppIcon::Ui(UiIcon::LIGHTNING),
            },
        ),
        ApiRequestTab::Headers => {
            let count = request.map_or(0, |request| {
                request
                    .headers
                    .iter()
                    .filter(|header| header.enabled)
                    .count()
            });
            (
                format!("{} ({count})", rust_i18n::t!("api.request.headers"),),
                AppIcon::Ui(UiIcon::LIST_FILES),
            )
        }
        ApiRequestTab::Auth => {
            let count = request.map_or(0, |request| {
                usize::from(!matches!(request.auth, ApiAuth::None))
            });
            (
                format!("{} ({count})", rust_i18n::t!("api.request.auth")),
                AppIcon::Ui(UiIcon::KEY),
            )
        }
        ApiRequestTab::Assertions => {
            let count = request.map_or(0, |request| request.assertions.len());
            (
                format!("{} ({count})", rust_i18n::t!("api.request.assertions")),
                AppIcon::Ui(UiIcon::CHECK),
            )
        }
    };
    Tab::new()
        .label(label)
        .prefix(AppIconElement::new(icon).xsmall())
}

fn render_body(params: &ApiWorkspaceParams<'_>, p: &Palette) -> AnyElement {
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

fn render_text_panel(
    id: &'static str,
    state: Entity<gpui_kit::component::input::TextareaState>,
    p: &Palette,
) -> AnyElement {
    div()
        .id(id)
        .flex_1()
        .min_h_0()
        .w_full()
        .p(rems(space::SM))
        .border_color(p.divider(p.border))
        .child(Textarea::new(&state).size_full())
        .into_any_element()
}

fn render_empty_request(p: &Palette) -> AnyElement {
    v_flex()
        .flex_1()
        .min_h_0()
        .items_center()
        .justify_center()
        .gap(rems(space::SM))
        .text_color(p.muted_foreground)
        .child(AppIconElement::new(AppIcon::Ui(UiIcon::WEB_SERVER)).small())
        .child(rust_i18n::t!("api.request.empty_title").to_string())
        .child(rust_i18n::t!("api.request.empty_hint").to_string())
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
