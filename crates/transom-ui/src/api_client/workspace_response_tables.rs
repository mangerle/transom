//! API 响应 Headers、Cookies、Assertions 与 History 表格。

use super::workspace_response_view::{centered_message, response_color};
use super::workspace_view::ApiWorkspaceParams;
use crate::palette::Palette;
use crate::theme::tokens::{size, space, typography};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::StyledExt;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_api::{ApiAssertionResult, ApiCookie, ApiResponseSnapshot};
use transom_ui_icons::{AppIcon, AppIconElement, UiIcon};

pub(super) fn render_headers(params: &ApiWorkspaceParams<'_>, p: &Palette) -> AnyElement {
    let Some(response) = params.response else {
        return centered_message(rust_i18n::t!("api.response.no_headers").to_string(), p);
    };
    if response.headers.is_empty() {
        return centered_message(rust_i18n::t!("api.response.no_headers").to_string(), p);
    }
    let border = p.divider(p.border);
    let header = h_flex()
        .h(rems(size::LARGE_HEADER_HEIGHT))
        .bg(p.list_head)
        .text_size(rems(typography::CAPTION))
        .text_color(p.muted_foreground)
        .border_color(border)
        .border_b_1()
        .child(
            div()
                .w(relative(0.28))
                .flex_none()
                .min_w_0()
                .h_full()
                .px(rems(space::SM))
                .flex()
                .items_center()
                .child(rust_i18n::t!("api.request.parameter_name").to_string()),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .h_full()
                .px(rems(space::SM))
                .flex()
                .items_center()
                .border_l_1()
                .border_color(border)
                .child(rust_i18n::t!("api.request.parameter_value").to_string()),
        );
    let rows = response
        .headers
        .iter()
        .enumerate()
        .map(|(index, (name, value))| {
            h_flex()
                .id(("api-response-header", index))
                .h(rems(size::LARGE_HEADER_HEIGHT))
                .items_center()
                .border_color(border)
                .border_b_1()
                .child(
                    div()
                        .w(relative(0.28))
                        .flex_none()
                        .min_w_0()
                        .h_full()
                        .px(rems(space::SM))
                        .flex()
                        .items_center()
                        .truncate()
                        .child(name.clone()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .px(rems(space::SM))
                        .flex()
                        .items_center()
                        .border_l_1()
                        .border_color(border)
                        .truncate()
                        .child(value.clone()),
                )
                .into_any_element()
        });
    v_flex()
        .id("api-response-headers")
        .flex_1()
        .min_h_0()
        .w_full()
        .p(rems(space::SM))
        .child(
            div()
                .flex_1()
                .min_h_0()
                .w_full()
                .overflow_y_scrollbar()
                .rounded(p.radius.md)
                .border_1()
                .border_color(border)
                .overflow_hidden()
                .child(header)
                .children(rows),
        )
        .into_any_element()
}

pub(super) fn render_cookies(response: Option<&ApiResponseSnapshot>, p: &Palette) -> AnyElement {
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

pub(super) fn render_assertions(results: &[ApiAssertionResult], p: &Palette) -> AnyElement {
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

pub(super) fn render_history(params: &ApiWorkspaceParams<'_>, p: &Palette) -> AnyElement {
    if params.project.history.is_empty() {
        return centered_message(rust_i18n::t!("api.response.no_history").to_string(), p);
    }
    let border = p.divider(p.border);
    let header = h_flex()
        .h(rems(size::LARGE_HEADER_HEIGHT))
        .bg(p.list_head)
        .text_size(rems(typography::CAPTION))
        .text_color(p.muted_foreground)
        .border_color(border)
        .border_b_1()
        .child(table_header_cell(
            rust_i18n::t!("api.response.history_time").to_string(),
            rems(5.5),
            border,
        ))
        .child(table_header_cell(
            rust_i18n::t!("api.response.history_status").to_string(),
            rems(4.0),
            border,
        ))
        .child(table_header_cell(
            rust_i18n::t!("api.response.history_method").to_string(),
            rems(5.0),
            border,
        ))
        .child(table_header_cell(
            rust_i18n::t!("api.response.history_url").to_string(),
            rems(0.0),
            border,
        ))
        .child(table_header_cell(
            rust_i18n::t!("api.response.history_elapsed").to_string(),
            rems(5.0),
            border,
        ))
        .child(table_header_cell(
            rust_i18n::t!("api.response.history_assertions").to_string(),
            rems(5.5),
            border,
        ));
    let rows = params
        .project
        .history
        .iter()
        .rev()
        .enumerate()
        .map(|(index, entry)| {
            let view = params.view.clone();
            let request_id = entry.request_id;
            let status_color = response_color(entry.status, p);
            h_flex()
                .id(("api-history-row", index))
                .h(rems(size::LARGE_HEADER_HEIGHT))
                .items_center()
                .border_color(border)
                .border_b_1()
                .hover(|row| row.bg(p.list_hover))
                .on_click(move |_, window, cx| {
                    view.update(cx, |this, cx| this.select_request(request_id, window, cx));
                })
                .child(table_cell(
                    history_time(entry.at),
                    rems(5.5),
                    p.muted_foreground,
                    border,
                    false,
                ))
                .child(table_cell(
                    entry.status.to_string(),
                    rems(4.0),
                    status_color,
                    border,
                    false,
                ))
                .child(table_cell(
                    entry.method.clone(),
                    rems(5.0),
                    p.foreground,
                    border,
                    false,
                ))
                .child(table_cell(
                    entry.url.clone(),
                    rems(0.0),
                    p.muted_foreground,
                    border,
                    true,
                ))
                .child(table_cell(
                    format!("{} ms", entry.elapsed_millis),
                    rems(5.0),
                    p.muted_foreground,
                    border,
                    false,
                ))
                .child(table_cell(
                    format!("{}/{}", entry.assertions_passed, entry.assertions_total),
                    rems(5.5),
                    if entry.assertions_passed == entry.assertions_total {
                        p.success
                    } else {
                        p.warning
                    },
                    border,
                    false,
                ))
                .into_any_element()
        });
    v_flex()
        .id("api-response-history")
        .flex_1()
        .min_h_0()
        .w_full()
        .p(rems(space::SM))
        .child(
            div()
                .flex_1()
                .min_h_0()
                .w_full()
                .overflow_y_scrollbar()
                .rounded(p.radius.md)
                .border_1()
                .border_color(border)
                .overflow_hidden()
                .child(header)
                .children(rows),
        )
        .into_any_element()
}

fn table_header_cell(label: String, width: Rems, border: Hsla) -> AnyElement {
    div()
        .w(width)
        .flex_none()
        .min_w_0()
        .h_full()
        .px(rems(space::SM))
        .flex()
        .items_center()
        .border_l_1()
        .border_color(border)
        .child(label)
        .into_any_element()
}

fn table_cell(text: String, width: Rems, color: Hsla, border: Hsla, flexible: bool) -> AnyElement {
    div()
        .when(flexible, |cell| cell.flex_1().min_w_0())
        .when(!flexible, |cell| cell.w(width).flex_none())
        .h_full()
        .px(rems(space::SM))
        .flex()
        .items_center()
        .text_color(color)
        .border_l_1()
        .border_color(border)
        .truncate()
        .child(text)
        .into_any_element()
}

fn history_time(at: u64) -> String {
    chrono::DateTime::from_timestamp(at as i64, 0)
        .map(|time| {
            time.with_timezone(&chrono::Local)
                .format("%m-%d %H:%M:%S")
                .to_string()
        })
        .unwrap_or_else(|| "--".to_string())
}
