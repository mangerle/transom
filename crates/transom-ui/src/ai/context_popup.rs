//! 上下文构成拆解弹窗（系统提示词、记忆、工具定义、历史消息、工具输出占比）与一键压缩。

use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::Button;
use gpui_kit::*;
use transom_core::AiContextBreakdownDto;

use super::metrics::format_with_commas;
use crate::palette::Palette;
use crate::theme::tokens::{space, typography};

/// 渲染上下文构成拆解分段条与各分项明细，并提供一键 `/compact` 触发按钮。
pub(super) fn render_context_breakdown_section(
    breakdown: &AiContextBreakdownDto,
    p: &Palette,
    on_compact: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    let total = breakdown
        .system_tokens
        .saturating_add(breakdown.memory_tokens)
        .saturating_add(breakdown.tools_tokens)
        .saturating_add(breakdown.history_tokens)
        .saturating_add(breakdown.tool_output_tokens)
        .max(1);

    let items = [
        (
            rust_i18n::t!("ai.ctx_part_system").to_string(),
            breakdown.system_tokens,
            p.primary,
        ),
        (
            rust_i18n::t!("ai.ctx_part_memory").to_string(),
            breakdown.memory_tokens,
            p.success,
        ),
        (
            rust_i18n::t!("ai.ctx_part_tools").to_string(),
            breakdown.tools_tokens,
            p.warning,
        ),
        (
            rust_i18n::t!("ai.ctx_part_history").to_string(),
            breakdown.history_tokens,
            p.foreground.opacity(0.65),
        ),
        (
            rust_i18n::t!("ai.ctx_part_outputs").to_string(),
            breakdown.tool_output_tokens,
            p.danger.opacity(0.8),
        ),
    ];

    let mut seg_bar = h_flex()
        .w_full()
        .h(px(6.0))
        .rounded(p.radius.sm)
        .overflow_hidden()
        .bg(p.secondary);
    for (_, val, color) in &items {
        let ratio = (*val as f32 / total as f32).clamp(0.0, 1.0);
        if ratio > 0.01 {
            seg_bar = seg_bar.child(div().h_full().w(relative(ratio)).bg(*color));
        }
    }

    let mut box_el = v_flex()
        .w_full()
        .pt(rems(space::XS))
        .gap(px(4.0))
        .border_t_1()
        .border_color(p.divider(p.border))
        .child(
            div()
                .text_size(rems(typography::CAPTION))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(p.text.muted)
                .child(rust_i18n::t!("ai.ctx_breakdown_title").to_string()),
        )
        .child(seg_bar);

    for (label, val, color) in items {
        box_el = box_el.child(
            h_flex()
                .w_full()
                .justify_between()
                .items_center()
                .child(
                    h_flex()
                        .gap(px(5.0))
                        .items_center()
                        .child(div().size(px(7.0)).rounded_full().bg(color))
                        .child(
                            div()
                                .text_size(rems(typography::CAPTION))
                                .text_color(p.text.muted)
                                .child(label),
                        ),
                )
                .child(
                    div()
                        .text_size(rems(typography::CAPTION))
                        .text_color(p.foreground)
                        .child(format!("{} tok", format_with_commas(u64::from(val)))),
                ),
        );
    }

    box_el = box_el.child(
        div().pt(px(4.0)).child(
            Button::new("ai-ctx-compact-now")
                .outline()
                .xsmall()
                .w_full()
                .label(rust_i18n::t!("ai.compact_now").to_string())
                .on_click(move |_, w, cx| on_compact(w, cx)),
        ),
    );
    box_el.into_any_element()
}
