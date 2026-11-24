//! API 请求编辑器：方法与环境地址栏、请求页签和协议主体。

use super::workspace_body_view::render_body;
use super::workspace_view::ApiWorkspaceParams;
use super::{ApiRequestTab, ApiWorkspace};
use crate::palette::Palette;
use crate::theme::tokens::{size, space};
use crate::widgets::{IconButton, IconButtonSize, action_button, dropdown};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Disableable;
use gpui_kit::component::Sizable;
use gpui_kit::component::button::ButtonVariants;
use gpui_kit::component::input::{Input, Textarea};
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::*;
use transom_api::{ApiAuth, ApiEnvironmentId, ApiMethod, ApiProtocol};
use transom_ui_icons::{AppIcon, AppIconElement, UiIcon};

#[derive(Clone, Debug, PartialEq, Eq)]
enum RequestMode {
    Http(ApiMethod),
    GraphQl,
    Grpc,
    WebSocket,
}

impl RequestMode {
    fn current(protocol: ApiProtocol, method: ApiMethod) -> Self {
        match protocol {
            ApiProtocol::Http => Self::Http(method),
            ApiProtocol::GraphQl => Self::GraphQl,
            ApiProtocol::Grpc => Self::Grpc,
            ApiProtocol::WebSocket => Self::WebSocket,
        }
    }

    fn pair(&self) -> (ApiProtocol, ApiMethod) {
        match self {
            Self::Http(method) => (ApiProtocol::Http, *method),
            Self::GraphQl => (ApiProtocol::GraphQl, ApiMethod::Post),
            Self::Grpc => (ApiProtocol::Grpc, ApiMethod::Post),
            Self::WebSocket => (ApiProtocol::WebSocket, ApiMethod::Get),
        }
    }
}

pub(super) fn render_request_editor(params: &ApiWorkspaceParams<'_>, p: &Palette) -> AnyElement {
    let view = params.view;
    let current_mode = RequestMode::current(params.protocol, params.method);
    let mut mode_options = ApiMethod::ALL
        .into_iter()
        .map(|method| (RequestMode::Http(method), method.as_str().to_string()))
        .collect::<Vec<_>>();
    mode_options.extend([
        (RequestMode::GraphQl, "GraphQL".to_string()),
        (
            RequestMode::Grpc,
            rust_i18n::t!("api.protocol.grpc_raw").to_string(),
        ),
        (RequestMode::WebSocket, "WebSocket".to_string()),
    ]);
    let mode = dropdown(
        "api-request-mode",
        current_mode,
        mode_options,
        |this: &mut ApiWorkspace, mode, _, cx| {
            let (protocol, method) = mode.pair();
            this.set_request_mode(protocol, method, cx);
        },
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
    let send_view = view.clone();
    let paste_view = view.clone();

    let mut content = v_flex()
        .size_full()
        .min_h_0()
        .overflow_hidden()
        .bg(p.background)
        .px(rems(space::SM))
        .pt(rems(space::SM))
        .child(
            h_flex()
                .h(rems(size::PANE_HEADER_HEIGHT))
                .w_full()
                .px(rems(space::XS))
                .gap(rems(space::SM))
                .items_center()
                .child(
                    div()
                        .flex_none()
                        .text_color(p.muted_foreground)
                        .child(rust_i18n::t!("api.request.name").to_string()),
                )
                .child(Input::new(params.request_name).w(rems(18.0))),
        )
        .child(
            h_flex()
                .h(rems(size::LARGE_HEADER_HEIGHT))
                .w_full()
                .px(rems(space::XS))
                .gap(rems(space::XS))
                .items_center()
                .rounded(p.radius.lg)
                .border_1()
                .border_color(p.divider(p.border))
                .child(mode)
                .child(divider(p))
                .child(environment)
                .child(Input::new(params.request_url).flex_1().min_w_0().on_paste(
                    move |clipboard, window, cx| {
                        let Some(text) = clipboard.text() else {
                            return false;
                        };
                        paste_view.update(cx, |this, cx| this.import_curl(&text, window, cx))
                    },
                ))
                .child(
                    action_button(
                        "api-request-send",
                        AppIcon::Ui(UiIcon::FETCH),
                        if params.protocol == ApiProtocol::WebSocket {
                            rust_i18n::t!("api.request.websocket_connect").to_string()
                        } else {
                            rust_i18n::t!("api.request.send").to_string()
                        },
                    )
                    .primary()
                    .disabled(params.selected_request.is_none() || params.sending)
                    .on_click(move |_, window, cx| {
                        send_view.update(cx, |this, cx| this.send_request(window, cx));
                    }),
                ),
        );

    if let Some(error) = params.error {
        content = content.child(message_bar(error.to_string(), p.danger, p));
    }
    if let Some(notice) = params.notice {
        content = content.child(message_bar(notice.to_string(), p.success, p));
    }

    let editor = if params.auxiliary_section.is_some() {
        super::workspace_advanced_view::render_auxiliary_panel(params, p)
    } else if params.selected_request.is_some() {
        render_tabbed_request(params, p)
    } else {
        render_empty_request(p)
    };
    content.child(editor).into_any_element()
}

fn render_tabbed_request(params: &ApiWorkspaceParams<'_>, p: &Palette) -> AnyElement {
    let view = params.view.clone();
    let ratio_view = params.view.clone();
    let save_view = params.view.clone();
    let tabs = visible_tabs(params);
    let selected = tabs
        .iter()
        .position(|tab| *tab == params.request_tab)
        .unwrap_or(0);
    let labels = tabs
        .iter()
        .copied()
        .map(|tab| request_tab(tab, params))
        .collect::<Vec<_>>();
    let click_tabs = tabs.clone();
    let bar = TabBar::new("api-request-tabs")
        .underline()
        .small()
        .w_full()
        .selected_index(selected)
        .children(labels)
        .suffix(
            h_flex()
                .gap(rems(space::XXS))
                .items_center()
                .child(dropdown(
                    "api-request-ratio",
                    params.editor_ratio,
                    vec![
                        (0.3, "30%".to_string()),
                        (0.5, "50%".to_string()),
                        (0.7, "70%".to_string()),
                    ],
                    |this: &mut ApiWorkspace, ratio, window, cx| {
                        this.set_editor_ratio(ratio, window, cx);
                    },
                    ratio_view,
                ))
                .child(
                    IconButton::new(
                        "api-request-save",
                        AppIcon::Ui(UiIcon::SAVE),
                        rust_i18n::t!("api.request.save").to_string(),
                    )
                    .size(IconButtonSize::Compact)
                    .render()
                    .on_click(move |_, _, cx| {
                        save_view.update(cx, |this, cx| this.save_project(cx));
                    }),
                )
                .child(super::workspace_advanced_view::render_project_actions_menu(
                    params,
                )),
        )
        .on_click(move |index, _, cx| {
            if let Some(tab) = click_tabs.get(*index).copied() {
                view.update(cx, |this, cx| this.set_request_tab(tab, cx));
            }
        });
    let content = match tabs.get(selected).copied().unwrap_or(ApiRequestTab::Query) {
        ApiRequestTab::Query => params.query_editor.clone().into_any_element(),
        ApiRequestTab::Body => render_body(params, p),
        ApiRequestTab::Headers => params.header_editor.clone().into_any_element(),
        ApiRequestTab::Auth => params.auth.clone().into_any_element(),
        ApiRequestTab::Assertions => params.assertions.clone().into_any_element(),
        ApiRequestTab::Settings => render_settings(params, p),
        ApiRequestTab::Docs => render_text_panel("api-request-docs", params.docs.clone(), p),
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

fn visible_tabs(params: &ApiWorkspaceParams<'_>) -> Vec<ApiRequestTab> {
    let mut tabs = Vec::with_capacity(5);
    if params.protocol == ApiProtocol::Http || params.protocol == ApiProtocol::GraphQl {
        tabs.push(ApiRequestTab::Query);
    }
    if params.protocol != ApiProtocol::Http || params.method.has_body() {
        tabs.push(ApiRequestTab::Body);
    }
    tabs.push(ApiRequestTab::Headers);
    tabs.push(ApiRequestTab::Auth);
    tabs.push(ApiRequestTab::Assertions);
    tabs.push(ApiRequestTab::Settings);
    tabs.push(ApiRequestTab::Docs);
    tabs
}

fn request_tab(tab: ApiRequestTab, params: &ApiWorkspaceParams<'_>) -> Tab {
    let request = params.request;
    let label = match tab {
        ApiRequestTab::Query => {
            let count = request.map_or(0, |request| {
                request
                    .query
                    .iter()
                    .filter(|param| param.enabled && !param.name.trim().is_empty())
                    .count()
            });
            labeled(rust_i18n::t!("api.request.query").to_string(), count)
        }
        ApiRequestTab::Body => match params.protocol {
            ApiProtocol::Http => rust_i18n::t!("api.request.body").to_string(),
            ApiProtocol::GraphQl => rust_i18n::t!("api.request.graphql_query").to_string(),
            ApiProtocol::Grpc => rust_i18n::t!("api.request.grpc_message").to_string(),
            ApiProtocol::WebSocket => rust_i18n::t!("api.request.websocket_messages").to_string(),
        },
        ApiRequestTab::Headers => {
            let count = request.map_or(0, |request| {
                request
                    .headers
                    .iter()
                    .filter(|header| header.enabled && !header.name.trim().is_empty())
                    .count()
            });
            labeled(rust_i18n::t!("api.request.headers").to_string(), count)
        }
        ApiRequestTab::Auth => {
            let count = request.map_or(0, |request| {
                usize::from(!matches!(request.auth, ApiAuth::None))
            });
            labeled(rust_i18n::t!("api.request.auth").to_string(), count)
        }
        ApiRequestTab::Assertions => {
            let count = request.map_or(0, |request| {
                request
                    .assertions
                    .iter()
                    .filter(|assertion| assertion.enabled)
                    .count()
            });
            labeled(rust_i18n::t!("api.request.assertions").to_string(), count)
        }
        ApiRequestTab::Settings => rust_i18n::t!("api.request.settings").to_string(),
        ApiRequestTab::Docs => rust_i18n::t!("api.request.docs").to_string(),
    };
    Tab::new().label(label)
}

fn labeled(name: String, count: usize) -> String {
    if count == 0 {
        name
    } else {
        format!("{name} ({count})")
    }
}

fn render_settings(params: &ApiWorkspaceParams<'_>, p: &Palette) -> AnyElement {
    v_flex()
        .id("api-request-settings")
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
                        .w(rems(9.0))
                        .text_color(p.muted_foreground)
                        .child(rust_i18n::t!("api.request.timeout").to_string()),
                )
                .child(Input::new(params.timeout).w(rems(10.0)))
                .child(
                    div()
                        .flex_1()
                        .text_color(p.muted_foreground)
                        .text_size(rems(crate::theme::tokens::typography::CAPTION))
                        .child(rust_i18n::t!("api.request.timeout_hint").to_string()),
                ),
        )
        .into_any_element()
}

pub(super) fn render_text_panel(
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

fn message_bar(message: String, color: Hsla, p: &Palette) -> AnyElement {
    div()
        .mx(rems(space::SM))
        .mt(rems(space::SM))
        .p(rems(space::SM))
        .rounded(p.radius.md)
        .bg(p.surface.control_selected)
        .text_color(color)
        .child(message)
        .into_any_element()
}

fn divider(p: &Palette) -> AnyElement {
    div()
        .w(px(1.0))
        .h(rems(1.5))
        .bg(p.divider(p.border))
        .into_any_element()
}
