//! API 请求历史列表与单条响应摘要详情。

use super::response_tables::table_cell;
use super::response_view::{centered_message, response_color};
use super::view::ApiWorkspaceParams;
use crate::api_client::editors::table::{ColumnWidth, table_header_row};
use crate::palette::Palette;
use crate::theme::tokens::{size, space};
use crate::widgets::{IconButton, IconButtonSize, action_button};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::ButtonVariants;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_api::ApiHistoryEntry;
use transom_ui_icons::{AppIcon, UiIcon};

pub(super) fn render_history(params: &ApiWorkspaceParams<'_>, p: &Palette) -> AnyElement {
    if params.project.history.is_empty() {
        return centered_message(rust_i18n::t!("api.response.no_history").to_string(), p);
    }
    let border = p.divider(p.border);
    let header = history_header(border, p);
    let history = params.project.history.clone();
    let total = history.len();
    let view = params.view.clone();
    let selected_history = params.ui.selected_history;
    let palette = p.clone();
    let list = gpui::uniform_list("api-response-history-list", total, move |range, _, _| {
        range
            .map(|display_index| {
                let actual_index = total.saturating_sub(1).saturating_sub(display_index);
                let entry = &history[actual_index];
                history_row(HistoryRowParams {
                    view: &view,
                    selected_history,
                    entry,
                    reverse_index: display_index,
                    total,
                    border,
                    p: &palette,
                })
            })
            .collect()
    })
    .flex_1()
    .w_full()
    .min_h_0();
    let clear_view = params.view.clone();
    let toolbar = h_flex()
        .w_full()
        .pb(rems(space::XS))
        .items_center()
        .child(div().flex_1())
        .child(
            action_button(
                "api-history-clear",
                AppIcon::Ui(UiIcon::DELETE),
                rust_i18n::t!("api.response.clear_history").to_string(),
            )
            .ghost()
            .on_click(move |_, _, cx| {
                clear_view.update(cx, |this, cx| this.clear_history(cx));
            }),
        );
    let mut content = v_flex()
        .id("api-response-history")
        .flex_1()
        .min_h_0()
        .w_full()
        .p(rems(space::SM))
        .child(toolbar)
        .child(
            v_flex()
                .flex_1()
                .min_h_0()
                .w_full()
                .rounded(p.radius.md)
                .border_1()
                .border_color(border)
                .overflow_hidden()
                .child(header)
                .child(list),
        );
    if let Some(entry) = params
        .ui
        .selected_history
        .and_then(|index| params.project.history.get(index))
    {
        content = content.child(render_history_detail(params, entry, p));
    }
    content.into_any_element()
}

fn history_header(border: Hsla, p: &Palette) -> AnyElement {
    table_header_row(
        border,
        p.list_head,
        p.muted_foreground,
        false,
        false,
        vec![
            (
                rust_i18n::t!("api.response.history_time").to_string(),
                ColumnWidth::Fixed(rems(8.5)),
            ),
            (
                rust_i18n::t!("api.response.history_status").to_string(),
                ColumnWidth::Fixed(rems(4.0)),
            ),
            (
                rust_i18n::t!("api.response.history_method").to_string(),
                ColumnWidth::Fixed(rems(5.0)),
            ),
            (
                rust_i18n::t!("api.response.history_url").to_string(),
                ColumnWidth::Flex,
            ),
            (
                rust_i18n::t!("api.response.history_elapsed").to_string(),
                ColumnWidth::Fixed(rems(5.0)),
            ),
            (
                rust_i18n::t!("api.response.history_assertions").to_string(),
                ColumnWidth::Fixed(rems(5.5)),
            ),
            (
                rust_i18n::t!("api.response.history_action").to_string(),
                ColumnWidth::Fixed(rems(6.0)),
            ),
        ],
    )
}

struct HistoryRowParams<'a> {
    view: &'a Entity<super::ApiWorkspace>,
    selected_history: Option<usize>,
    entry: &'a ApiHistoryEntry,
    reverse_index: usize,
    total: usize,
    border: Hsla,
    p: &'a Palette,
}

fn history_row(params: HistoryRowParams<'_>) -> AnyElement {
    let HistoryRowParams {
        view,
        selected_history,
        entry,
        reverse_index,
        total,
        border,
        p,
    } = params;
    let actual_index = total.saturating_sub(1).saturating_sub(reverse_index);
    let selected = selected_history == Some(actual_index);
    let view = view.clone();
    let replay_view = view.clone();
    let request_id = entry.request_id;
    let status_color = response_color(entry.status, p);
    h_flex()
        .id(("api-history-row", reverse_index))
        .h(rems(size::LARGE_HEADER_HEIGHT))
        .items_center()
        .border_color(border)
        .border_b_1()
        .when(selected, |row| row.bg(p.list_active))
        .hover(|row| row.bg(p.list_hover))
        .on_click(move |_, window, cx| {
            view.update(cx, |this, cx| {
                this.select_history(actual_index, request_id, window, cx)
            });
        })
        .child(table_cell(
            history_time(entry.at),
            rems(8.5),
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
        .child(
            div()
                .w(rems(6.0))
                .flex_none()
                .h_full()
                .px(rems(space::XS))
                .flex()
                .items_center()
                .justify_center()
                .border_l_1()
                .border_color(border)
                .child(
                    action_button(
                        ("api-history-replay", reverse_index),
                        AppIcon::Ui(UiIcon::FETCH),
                        rust_i18n::t!("api.response.history_replay_short").to_string(),
                    )
                    .ghost()
                    .on_click(move |_, window, cx| {
                        cx.stop_propagation();
                        replay_view.update(cx, |this, cx| {
                            this.replay_history(actual_index, request_id, window, cx)
                        });
                    }),
                ),
        )
        .into_any_element()
}

fn render_history_detail(
    params: &ApiWorkspaceParams<'_>,
    entry: &ApiHistoryEntry,
    p: &Palette,
) -> AnyElement {
    let close_view = params.view.clone();
    let excerpt = if entry.response_excerpt.trim().is_empty() {
        rust_i18n::t!("api.response.no_response_excerpt").to_string()
    } else {
        entry.response_excerpt.clone()
    };
    v_flex()
        .h(rems(9.0))
        .flex_none()
        .gap(rems(space::XXS))
        .border_color(p.divider(p.border))
        .border_t_1()
        .child(
            h_flex()
                .h(rems(size::PANE_HEADER_HEIGHT))
                .px(rems(space::SM))
                .gap(rems(space::SM))
                .items_center()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .child(entry.request_name.clone()),
                )
                .child(
                    div()
                        .text_color(response_color(entry.status, p))
                        .child(entry.status.to_string()),
                )
                .child(
                    div()
                        .text_color(p.muted_foreground)
                        .child(history_time(entry.at)),
                )
                .child(
                    IconButton::new(
                        "api-history-detail-close",
                        AppIcon::Ui(UiIcon::CLOSE),
                        rust_i18n::t!("api.response.close_history_detail").to_string(),
                    )
                    .size(IconButtonSize::RowAction)
                    .render()
                    .on_click(move |_, _, cx| {
                        close_view.update(cx, |this, cx| this.clear_history_selection(cx));
                    }),
                ),
        )
        .child(
            h_flex()
                .px(rems(space::SM))
                .gap(rems(space::SM))
                .text_color(p.muted_foreground)
                .child(entry.method.clone())
                .child(div().flex_1().min_w_0().truncate().child(entry.url.clone()))
                .child(format!("{} ms", entry.elapsed_millis)),
        )
        .child(
            div()
                .flex_1()
                .min_h_0()
                .mx(rems(space::SM))
                .mb(rems(space::XS))
                .p(rems(space::XS))
                .overflow_y_scrollbar()
                .rounded(p.radius.sm)
                .bg(p.surface.control_selected)
                .font_family(p.mono_font_family.clone())
                .child(excerpt),
        )
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
