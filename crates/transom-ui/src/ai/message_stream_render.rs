//! AI 消息流辅助渲染组件：运行态焦点行与错误提示横幅。

use gpui_kit::base::h_flex;
use gpui_kit::*;
use transom_ui_icons::{AppIcon, AppIconElement, UiIcon};

use super::metrics::format_duration_zh;
use super::work_group::format_running_focus;
use crate::palette::Palette;
use crate::theme::tokens::{space, typography};

/// 渲染底部运行态焦点与耗时行。
pub(in crate::app::ai) fn render_running_focus_row(
    active_focus: Option<&str>,
    live_elapsed: Option<u64>,
    p: &Palette,
) -> AnyElement {
    let focus = active_focus
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| rust_i18n::t!("ai.working_default").to_string());
    let elapsed_str = live_elapsed
        .filter(|&s| s > 0)
        .map(|s| format_duration_zh(s.saturating_mul(1000)))
        .unwrap_or_default();
    let label = format_running_focus(&focus);
    let mut row = h_flex()
        .w_full()
        .min_w_0()
        .py(px(4.0))
        .gap(rems(space::XS))
        .justify_between()
        .items_center()
        .child(
            h_flex()
                .flex_1()
                .min_w_0()
                .gap(px(6.0))
                .items_center()
                .overflow_hidden()
                .child(
                    div()
                        .flex_shrink_0()
                        .text_color(p.primary)
                        .child(AppIconElement::new(AppIcon::Ui(UiIcon::REFRESH)).small()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(rems(typography::CAPTION))
                        .text_color(p.text.muted)
                        .child(label),
                ),
        );
    if !elapsed_str.is_empty() {
        row = row.child(
            div()
                .flex_shrink_0()
                .text_size(rems(typography::CAPTION))
                .text_color(p.text.muted)
                .child(elapsed_str),
        );
    }
    row.into_any_element()
}

/// 渲染消息流底部的错误提示横幅。
pub(in crate::app::ai) fn render_status_error_banner(err: &str, p: &Palette) -> AnyElement {
    h_flex()
        .w_full()
        .min_w_0()
        .px(rems(space::SM))
        .py(rems(space::XS))
        .gap(px(6.0))
        .items_start()
        .rounded(p.radius.md)
        .bg(p.secondary)
        .border_1()
        .border_color(p.danger)
        .child(
            div()
                .flex_shrink_0()
                .mt(px(2.0))
                .text_color(p.danger)
                .child(AppIconElement::new(AppIcon::Ui(UiIcon::ERROR)).small()),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(rems(typography::CAPTION))
                .text_color(p.danger)
                .child(err.to_owned()),
        )
        .into_any_element()
}
