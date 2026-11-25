//! API 响应面板：摘要、响应级页签、正文、头、Cookie、断言与历史。

use super::ApiResponseTab;
use super::view::ApiWorkspaceParams;
use crate::palette::Palette;
use crate::theme::tokens::space;
use crate::widgets::{IconButton, IconButtonSize};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::input::Textarea;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_api::{ApiProtocol, ApiResponseSnapshot};
use transom_ui_icons::{AppIcon, UiIcon};

const RESPONSE_TABS: [ApiResponseTab; 5] = [
    ApiResponseTab::Body,
    ApiResponseTab::Headers,
    ApiResponseTab::Cookies,
    ApiResponseTab::Assertions,
    ApiResponseTab::History,
];

pub(super) fn render_response(params: &ApiWorkspaceParams<'_>, p: &Palette) -> AnyElement {
    let copy_view = params.view.clone();
    let suffix = h_flex()
        .gap(rems(space::SM))
        .items_center()
        .children(render_summary(params, p))
        .when(
            params.response.is_some() || params.protocol == ApiProtocol::WebSocket,
            |row| {
                row.child(
                    IconButton::new(
                        "api-response-copy-body",
                        AppIcon::Ui(UiIcon::COPY),
                        rust_i18n::t!("api.response.copy_body").to_string(),
                    )
                    .size(IconButtonSize::Compact)
                    .render()
                    .on_click(move |_, _, cx| {
                        copy_view.update(cx, |this, cx| this.copy_response_body(cx));
                    }),
                )
            },
        );
    let items = RESPONSE_TABS
        .into_iter()
        .map(|tab| {
            let view = params.view.clone();
            super::tabs_bar::UnderlineTabItem {
                id: response_tab_id(tab),
                label: response_tab_label(tab, params),
                selected: tab == params.response_tab,
                on_select: Box::new(move |_, cx| {
                    view.update(cx, |this, cx| this.set_response_tab(tab, cx));
                }),
            }
        })
        .collect::<Vec<_>>();
    let tab_bar = super::tabs_bar::render_underline_tabs(
        "api-response-tabs",
        items,
        Some(suffix.into_any_element()),
        p,
    );

    let mut body = v_flex()
        .size_full()
        .min_h_0()
        .overflow_hidden()
        .bg(p.background)
        .child(tab_bar);

    body = body.child(match params.response_tab {
        ApiResponseTab::Body => render_body(params, p, params.response),
        ApiResponseTab::Headers => super::response_tables::render_headers(params, p),
        ApiResponseTab::Cookies => super::response_tables::render_cookies(params.response, p),
        ApiResponseTab::Assertions => {
            super::response_tables::render_assertions(params.assertion_results, p)
        }
        ApiResponseTab::History => super::history_view::render_history(params, p),
    });
    body.into_any_element()
}

fn render_summary(params: &ApiWorkspaceParams<'_>, p: &Palette) -> Vec<AnyElement> {
    let Some(response) = params.response else {
        if params.sending {
            return vec![
                div()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("api.response.sending").to_string())
                    .into_any_element(),
            ];
        }
        return Vec::new();
    };
    let assertions = &params.assertion_results;
    vec![
        div()
            .text_color(response_color(response.status, p))
            .child(format!("{} {}", response.status, response.status_text))
            .into_any_element(),
        div()
            .text_color(p.muted_foreground)
            .child(
                rust_i18n::t!(
                    "api.response.metrics",
                    elapsed = response.elapsed_millis,
                    size = response.body_size
                )
                .to_string(),
            )
            .into_any_element(),
        div()
            .text_color(if assertions.iter().all(|result| result.passed) {
                p.success
            } else {
                p.warning
            })
            .child(
                rust_i18n::t!(
                    "api.response.assertions",
                    passed = assertions.iter().filter(|result| result.passed).count(),
                    total = assertions.len()
                )
                .to_string(),
            )
            .into_any_element(),
    ]
}

fn response_tab_id(tab: ApiResponseTab) -> &'static str {
    match tab {
        ApiResponseTab::Body => "api-resp-tab-body",
        ApiResponseTab::Headers => "api-resp-tab-headers",
        ApiResponseTab::Cookies => "api-resp-tab-cookies",
        ApiResponseTab::Assertions => "api-resp-tab-assertions",
        ApiResponseTab::History => "api-resp-tab-history",
    }
}

fn response_tab_label(tab: ApiResponseTab, params: &ApiWorkspaceParams<'_>) -> String {
    match tab {
        ApiResponseTab::Body => rust_i18n::t!("api.response.body").to_string(),
        ApiResponseTab::Headers => {
            let count = params.response.map_or(0, |response| response.headers.len());
            format!("{} ({count})", rust_i18n::t!("api.response.headers"))
        }
        ApiResponseTab::Cookies => {
            let count = params
                .response
                .map_or(0, |response| response.set_cookies.len());
            format!("{} ({count})", rust_i18n::t!("api.response.cookies"))
        }
        ApiResponseTab::Assertions => format!(
            "{} ({})",
            rust_i18n::t!("api.response.assertions_label"),
            params.assertion_results.len()
        ),
        ApiResponseTab::History => format!(
            "{} ({})",
            rust_i18n::t!("api.response.history"),
            params.project.history.len()
        ),
    }
}

fn render_body(
    params: &ApiWorkspaceParams<'_>,
    p: &Palette,
    response: Option<&ApiResponseSnapshot>,
) -> AnyElement {
    if params.protocol == ApiProtocol::WebSocket {
        return text_content("api-response-ws-log", params.websocket_log.clone(), p);
    }
    if let Some(_response) = response {
        return text_content("api-response-body", params.response_body.clone(), p);
    }
    if params.sending {
        return centered_message(rust_i18n::t!("api.response.sending").to_string(), p);
    }
    centered_message(rust_i18n::t!("api.response.empty").to_string(), p)
}

fn text_content(
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
        .child(Textarea::new(&state).readonly(true).size_full())
        .into_any_element()
}

pub(super) fn centered_message(message: String, p: &Palette) -> AnyElement {
    v_flex()
        .flex_1()
        .min_h_0()
        .items_center()
        .justify_center()
        .text_color(p.muted_foreground)
        .child(message)
        .into_any_element()
}

pub(super) fn response_color(status: u16, p: &Palette) -> Hsla {
    if status >= 500 {
        p.danger
    } else if status >= 400 {
        p.warning
    } else if (200..300).contains(&status) {
        p.success
    } else {
        p.muted_foreground
    }
}
