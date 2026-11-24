//! API 响应面板：摘要、响应级页签、正文、头、Cookie、断言与历史。

use super::ApiResponseTab;
use super::workspace_view::ApiWorkspaceParams;
use crate::palette::Palette;
use crate::theme::tokens::{size, space, typography};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::input::Textarea;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{Sizable, StyledExt};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_api::{ApiAssertionResult, ApiCookie, ApiProtocol, ApiResponseSnapshot};
use transom_ui_icons::{AppIcon, AppIconElement, CustomIcon, UiIcon};

const RESPONSE_TABS: [ApiResponseTab; 5] = [
    ApiResponseTab::Body,
    ApiResponseTab::Headers,
    ApiResponseTab::Cookies,
    ApiResponseTab::Assertions,
    ApiResponseTab::History,
];

pub(super) fn render_response(params: &ApiWorkspaceParams<'_>, p: &Palette) -> AnyElement {
    let view = params.view.clone();
    let selected = RESPONSE_TABS
        .iter()
        .position(|tab| *tab == params.response_tab)
        .unwrap_or(0);
    let tab_bar = TabBar::new("api-response-tabs")
        .underline()
        .small()
        .w_full()
        .selected_index(selected)
        .children(
            RESPONSE_TABS
                .into_iter()
                .map(|tab| response_tab(tab, params)),
        )
        .on_click(move |index, _, cx| {
            if let Some(tab) = RESPONSE_TABS.get(*index).copied() {
                view.update(cx, |this, cx| this.set_response_tab(tab, cx));
            }
        });

    let mut body = v_flex()
        .size_full()
        .min_h_0()
        .overflow_hidden()
        .bg(p.background)
        .child(
            h_flex()
                .h(rems(size::PANE_HEADER_HEIGHT))
                .px(rems(space::SM))
                .gap(rems(space::SM))
                .items_center()
                .border_color(p.divider(p.border))
                .border_b_1()
                .child(
                    div()
                        .flex_1()
                        .child(rust_i18n::t!("api.response.title").to_string()),
                )
                .children(render_summary(params, p)),
        )
        .child(
            div()
                .px(rems(space::SM))
                .border_color(p.divider(p.border))
                .border_b_1()
                .child(tab_bar),
        );

    body = body.child(match params.response_tab {
        ApiResponseTab::Body => render_body(params, p, params.response),
        ApiResponseTab::Headers => render_headers(params, p),
        ApiResponseTab::Cookies => render_cookies(params.response, p),
        ApiResponseTab::Assertions => render_assertions(params.assertion_results, p),
        ApiResponseTab::History => render_history(params, p),
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

fn response_tab(tab: ApiResponseTab, params: &ApiWorkspaceParams<'_>) -> Tab {
    let (label, icon) = match tab {
        ApiResponseTab::Body => (
            rust_i18n::t!("api.response.body").to_string(),
            AppIcon::Ui(UiIcon::EDITOR_PREVIEW),
        ),
        ApiResponseTab::Headers => {
            let count = params.response.map_or(0, |response| response.headers.len());
            (
                format!("{} ({count})", rust_i18n::t!("api.response.headers")),
                AppIcon::Ui(UiIcon::LIST_FILES),
            )
        }
        ApiResponseTab::Cookies => {
            let count = params
                .response
                .map_or(0, |response| response.set_cookies.len());
            (
                format!("{} ({count})", rust_i18n::t!("api.response.cookies")),
                AppIcon::Custom(CustomIcon::COOKIE),
            )
        }
        ApiResponseTab::Assertions => (
            format!(
                "{} ({})",
                rust_i18n::t!("api.response.assertions_label"),
                params.assertion_results.len()
            ),
            AppIcon::Ui(UiIcon::GREEN_CHECKMARK),
        ),
        ApiResponseTab::History => (
            format!(
                "{} ({})",
                rust_i18n::t!("api.response.history"),
                params.project.history.len()
            ),
            AppIcon::Ui(UiIcon::HISTORY),
        ),
    };
    Tab::new()
        .label(label)
        .prefix(AppIconElement::new(icon).xsmall())
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

fn render_headers(params: &ApiWorkspaceParams<'_>, p: &Palette) -> AnyElement {
    if params.response.is_some() {
        text_content("api-response-headers", params.response_headers.clone(), p)
    } else {
        centered_message(rust_i18n::t!("api.response.no_headers").to_string(), p)
    }
}

fn render_cookies(response: Option<&ApiResponseSnapshot>, p: &Palette) -> AnyElement {
    let Some(response) = response else {
        return centered_message(rust_i18n::t!("api.response.no_cookies").to_string(), p);
    };
    if response.set_cookies.is_empty() {
        return centered_message(rust_i18n::t!("api.response.no_cookies").to_string(), p);
    }
    let rows = response
        .set_cookies
        .iter()
        .map(|cookie| cookie_row(cookie, p));
    div()
        .id("api-response-cookies")
        .flex_1()
        .min_h_0()
        .w_full()
        .overflow_y_scrollbar()
        .p(rems(space::SM))
        .children(rows)
        .into_any_element()
}

fn cookie_row(cookie: &ApiCookie, p: &Palette) -> AnyElement {
    let attributes = [
        cookie
            .domain
            .as_ref()
            .map(|domain| format!("Domain={domain}")),
        (!cookie.path.is_empty()).then(|| format!("Path={}", cookie.path)),
        cookie.secure.then(|| "Secure".to_string()),
        cookie.http_only.then(|| "HttpOnly".to_string()),
        cookie
            .expires_at
            .map(|expires| format!("Expires={expires}")),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ");
    v_flex()
        .id(format!(
            "api-response-cookie-{}-{}",
            cookie.name, cookie.path
        ))
        .w_full()
        .gap(rems(space::XXS))
        .py(rems(space::XS))
        .border_color(p.divider(p.border))
        .border_b_1()
        .child(
            h_flex()
                .gap(rems(space::SM))
                .child(div().font_semibold().child(cookie.name.clone()))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_color(p.muted_foreground)
                        .child(cookie.value.clone()),
                ),
        )
        .when(!attributes.is_empty(), |row| {
            row.child(
                div()
                    .text_size(rems(typography::CAPTION))
                    .text_color(p.muted_foreground)
                    .child(attributes),
            )
        })
        .into_any_element()
}

fn render_assertions(results: &[ApiAssertionResult], p: &Palette) -> AnyElement {
    if results.is_empty() {
        return centered_message(rust_i18n::t!("api.response.no_assertions").to_string(), p);
    }
    v_flex()
        .id("api-response-assertions")
        .flex_1()
        .min_h_0()
        .w_full()
        .overflow_y_scrollbar()
        .p(rems(space::SM))
        .gap(rems(space::XS))
        .children(results.iter().map(|result| {
            h_flex()
                .gap(rems(space::XS))
                .items_center()
                .text_size(rems(typography::CAPTION))
                .text_color(if result.passed { p.success } else { p.danger })
                .child(
                    AppIconElement::new(AppIcon::Ui(if result.passed {
                        UiIcon::CHECK
                    } else {
                        UiIcon::ERROR
                    }))
                    .xsmall(),
                )
                .child(result.expression.clone())
                .child(
                    div()
                        .text_color(p.muted_foreground)
                        .child(result.detail.clone()),
                )
                .into_any_element()
        }))
        .into_any_element()
}

fn render_history(params: &ApiWorkspaceParams<'_>, p: &Palette) -> AnyElement {
    text_content("api-response-history", params.history_editor.clone(), p)
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

fn centered_message(message: String, p: &Palette) -> AnyElement {
    v_flex()
        .flex_1()
        .min_h_0()
        .items_center()
        .justify_center()
        .text_color(p.muted_foreground)
        .child(message)
        .into_any_element()
}

fn response_color(status: u16, p: &Palette) -> Hsla {
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
