//! AI 会话统计、Token 用量与上下文使用情况详情浮层渲染。

use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::progress::ProgressCircle;
use gpui_kit::*;
use transom_ui_icons::{AppIcon, AppIconElement, CustomIcon};

pub(super) use super::metrics_model::{
    AiSessionMetrics, format_compact_tokens, format_duration_zh, format_with_commas,
};
use super::state::AiActivePopup;
use crate::palette::Palette;
use crate::theme::tokens::{space, typography};

fn popup_shell(p: &Palette) -> Div {
    v_flex()
        .w_full()
        .p(rems(space::MD))
        .gap(rems(space::XS))
        .rounded(p.radius.md)
        .bg(p.surface.floating)
        .border_1()
        .border_color(p.border)
        .shadow_md()
}

fn metric_row(label: String, value: String, p: &Palette) -> AnyElement {
    h_flex()
        .w_full()
        .justify_between()
        .items_center()
        .child(
            div()
                .text_size(rems(typography::BODY))
                .text_color(p.text.muted)
                .child(label),
        )
        .child(
            div()
                .text_size(rems(typography::BODY))
                .text_color(p.foreground)
                .child(value),
        )
        .into_any_element()
}

/// 渲染“会话统计”详情浮层。
pub(super) fn render_session_stats_popup(m: &AiSessionMetrics, p: &Palette) -> AnyElement {
    let header_right =
        rust_i18n::t!("ai.stats_turns_steps", turns = m.turns, steps = m.steps).to_string();
    let ttft_str = m
        .avg_ttft_ms()
        .map(format_duration_zh)
        .unwrap_or_else(|| "--".to_owned());
    let tps_str = m
        .last_tps
        .map(|t| format!("{t:.0} tok/s"))
        .unwrap_or_else(|| "-- tok/s".to_owned());

    popup_shell(p)
        .child(
            h_flex()
                .w_full()
                .pb(rems(space::XS))
                .justify_between()
                .items_center()
                .border_b_1()
                .border_color(p.divider(p.border))
                .child(
                    h_flex()
                        .gap(rems(space::XS))
                        .items_center()
                        .child(div().text_color(p.foreground).child(
                            AppIconElement::new(AppIcon::Custom(CustomIcon::SPEEDOMETER)).small(),
                        ))
                        .child(
                            div()
                                .text_size(rems(typography::BODY))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(p.foreground)
                                .child(rust_i18n::t!("ai.stats_title").to_string()),
                        ),
                )
                .child(
                    div()
                        .text_size(rems(typography::BODY))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(p.foreground)
                        .child(header_right),
                ),
        )
        .child(metric_row(
            rust_i18n::t!("ai.stats_model_time").to_string(),
            format_duration_zh(m.model_duration_ms),
            p,
        ))
        .child(metric_row(
            rust_i18n::t!("ai.stats_tool_time").to_string(),
            format_duration_zh(m.tool_duration_ms),
            p,
        ))
        .child(metric_row(
            rust_i18n::t!("ai.stats_ttft").to_string(),
            ttft_str,
            p,
        ))
        .child(metric_row(
            rust_i18n::t!("ai.stats_tps").to_string(),
            tps_str,
            p,
        ))
        .into_any_element()
}

/// 渲染“Token 用量”详情浮层。
pub(super) fn render_token_usage_popup(m: &AiSessionMetrics, p: &Palette) -> AnyElement {
    let total_str = format!("{} tok", format_with_commas(m.total_tokens()));
    popup_shell(p)
        .child(
            h_flex()
                .w_full()
                .pb(rems(space::XS))
                .justify_between()
                .items_center()
                .border_b_1()
                .border_color(p.divider(p.border))
                .child(
                    h_flex()
                        .gap(rems(space::XS))
                        .items_center()
                        .child(div().text_color(p.foreground).child(
                            AppIconElement::new(AppIcon::Custom(CustomIcon::DATABASE)).small(),
                        ))
                        .child(
                            div()
                                .text_size(rems(typography::BODY))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(p.foreground)
                                .child(rust_i18n::t!("ai.usage_title").to_string()),
                        ),
                )
                .child(
                    div()
                        .text_size(rems(typography::BODY))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(p.foreground)
                        .child(total_str),
                ),
        )
        .child(metric_row(
            rust_i18n::t!("ai.usage_cache_hit").to_string(),
            format!("{}%", m.cache_hit_percent()),
            p,
        ))
        .child(metric_row(
            rust_i18n::t!("ai.usage_uncached_input").to_string(),
            format!("{} tok", format_with_commas(m.uncached_input_tokens())),
            p,
        ))
        .child(metric_row(
            rust_i18n::t!("ai.usage_cache_read").to_string(),
            format!("{} tok", format_with_commas(m.cached_input_tokens)),
            p,
        ))
        .child(metric_row(
            rust_i18n::t!("ai.usage_output").to_string(),
            format!("{} tok", format_with_commas(m.output_tokens)),
            p,
        ))
        .child(
            div()
                .w_full()
                .h(px(1.0))
                .my(px(4.0))
                .bg(p.divider(p.border)),
        )
        .child(
            h_flex()
                .w_full()
                .gap(rems(space::LG))
                .child(
                    v_flex()
                        .flex_1()
                        .gap(px(2.0))
                        .child(
                            div()
                                .text_size(rems(typography::CAPTION))
                                .text_color(p.text.muted)
                                .child(rust_i18n::t!("ai.usage_today").to_string()),
                        )
                        .child(
                            div()
                                .text_size(rems(typography::TITLE))
                                .font_weight(FontWeight::BOLD)
                                .text_color(p.foreground)
                                .child(format!("{} Token", format_compact_tokens(m.today_tokens))),
                        ),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .gap(px(2.0))
                        .child(
                            div()
                                .text_size(rems(typography::CAPTION))
                                .text_color(p.text.muted)
                                .child(rust_i18n::t!("ai.usage_month").to_string()),
                        )
                        .child(
                            div()
                                .text_size(rems(typography::TITLE))
                                .font_weight(FontWeight::BOLD)
                                .text_color(p.foreground)
                                .child(format!("{} Token", format_compact_tokens(m.month_tokens))),
                        ),
                ),
        )
        .into_any_element()
}

/// 渲染“上下文使用情况”详情浮层。
pub(super) fn render_context_usage_popup(
    m: &AiSessionMetrics,
    context_window: u32,
    msg_count: usize,
    p: &Palette,
    on_compact: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    let pct = m.context_percent(context_window);
    let win = u64::from(context_window.max(1));
    let remaining = win.saturating_sub(m.last_context_tokens);
    let bar_color = if pct >= 90 {
        p.danger
    } else if pct >= 75 {
        p.warning
    } else {
        p.primary
    };
    let ratio = (pct as f32 / 100.0).clamp(0.0, 1.0);

    let mut box_el = popup_shell(p)
        .child(
            h_flex()
                .w_full()
                .pb(rems(space::XS))
                .justify_between()
                .items_center()
                .border_b_1()
                .border_color(p.divider(p.border))
                .child(
                    h_flex()
                        .gap(rems(space::XS))
                        .items_center()
                        .child(
                            ProgressCircle::new("ai-ctx-popup-ring")
                                .value(pct as f32)
                                .size(px(14.0))
                                .color(bar_color),
                        )
                        .child(
                            div()
                                .text_size(rems(typography::BODY))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(p.foreground)
                                .child(rust_i18n::t!("ai.ctx_title").to_string()),
                        ),
                )
                .child(
                    div()
                        .text_size(rems(typography::BODY))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(bar_color)
                        .child(format!("{pct}%")),
                ),
        )
        .child(
            div()
                .w_full()
                .h(px(6.0))
                .rounded(p.radius.sm)
                .bg(p.secondary)
                .overflow_hidden()
                .child(div().h_full().w(relative(ratio)).bg(bar_color)),
        )
        .child(metric_row(
            rust_i18n::t!("ai.ctx_used").to_string(),
            format!("{} tok", format_with_commas(m.last_context_tokens)),
            p,
        ))
        .child(metric_row(
            rust_i18n::t!("ai.ctx_limit").to_string(),
            format!("{} tok", format_with_commas(win)),
            p,
        ))
        .child(metric_row(
            rust_i18n::t!("ai.ctx_remaining").to_string(),
            format!("{} tok", format_with_commas(remaining)),
            p,
        ))
        .child(metric_row(
            rust_i18n::t!("ai.ctx_messages").to_string(),
            msg_count.to_string(),
            p,
        ));
    if let Some(bd) = &m.last_breakdown {
        box_el = box_el.child(super::context_popup::render_context_breakdown_section(
            bd, p, on_compact,
        ));
    }
    box_el.into_any_element()
}

/// 将底部控制条弹层锚定到对应触发按钮正上方。
pub(super) fn anchor_toolbar_popover(kind: AiActivePopup, pop: AnyElement) -> AnyElement {
    let anchor = match kind {
        AiActivePopup::Backend => div().left(px(14.0)).w(px(230.0)),
        AiActivePopup::Mode => div().left(px(46.0)).w(px(280.0)),
        AiActivePopup::Model => div().left(px(84.0)).w(px(260.0)),
        AiActivePopup::SessionStats => div().left(px(175.0)).w(px(250.0)),
        AiActivePopup::TokenUsage => div().left(px(235.0)).w(px(270.0)),
        AiActivePopup::ContextUsage => div().right(px(52.0)).w(px(250.0)),
        AiActivePopup::TargetSession | AiActivePopup::None => div(),
    };
    deferred(
        anchor
            .absolute()
            .bottom(px(48.0))
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(pop),
    )
    .with_priority(1)
    .into_any_element()
}
