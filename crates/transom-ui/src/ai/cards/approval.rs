//! 悬浮在输入行上方的 AI 工具审批卡片。

use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::*;
use transom_core::{AiApprovalDecision, AiApprovalDto, HostProtection};
use transom_ui_icons::{AppIcon, AppIconElement, CustomIcon, UiIcon};

use crate::palette::Palette;
use crate::theme::tokens::{space, typography};

/// 渲染当前队首的待审批卡片（支持影响提示、前缀放行、移交终端与附带意见拒绝）。
pub(in crate::app::ai) fn render_approval_card(
    approval: &AiApprovalDto,
    feedback_input: &Entity<InputState>,
    p: &Palette,
    on_decide: impl Fn(AiApprovalDecision, &mut Window, &mut App) + Clone + 'static,
    cx: &App,
) -> AnyElement {
    let is_high_risk = approval.is_destructive
        || approval.requires_fresh_approval
        || approval.protection == HostProtection::Production;
    let accent = if is_high_risk { p.danger } else { p.warning };
    let header_icon = if is_high_risk {
        AppIcon::Ui(UiIcon::ERROR)
    } else {
        AppIcon::Custom(CustomIcon::SHIELD_CHECK)
    };

    let mut card = v_flex()
        .w_full()
        .p(rems(space::SM))
        .gap(rems(space::XS))
        .rounded(p.radius.md)
        .bg(p.surface.floating)
        .shadow_sm()
        .border_1()
        .border_color(p.divider(p.border))
        .border_l_4()
        .border_color(accent)
        .child(
            h_flex()
                .justify_between()
                .items_center()
                .gap(rems(space::XS))
                .child(
                    h_flex()
                        .flex_1()
                        .gap(px(6.0))
                        .items_center()
                        .overflow_hidden()
                        .child(
                            div()
                                .flex_shrink_0()
                                .text_color(accent)
                                .child(AppIconElement::new(header_icon).small()),
                        )
                        .child(
                            div()
                                .px(px(6.0))
                                .py(px(1.0))
                                .rounded_full()
                                .bg(p.secondary)
                                .text_size(rems(typography::CAPTION))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(accent)
                                .child(approval.risk_label.clone()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .truncate()
                                .text_size(rems(typography::CAPTION))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(p.foreground)
                                .child(format!("{} · {}", approval.host_label, approval.title)),
                        ),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .text_size(rems(typography::CAPTION))
                        .text_color(p.text.muted)
                        .child(approval.protection.label()),
                ),
        )
        .child(
            div()
                .id("ai-approval-detail")
                .max_h(rems(10.0))
                .overflow_y_scroll()
                .p(rems(space::XS))
                .rounded(p.radius.sm)
                .bg(p.background)
                .font_family("monospace")
                .text_size(rems(typography::CAPTION))
                .text_color(p.foreground)
                .child(approval.detail.clone()),
        );

    if approval.requires_fresh_approval {
        card = card.child(
            div()
                .text_size(rems(typography::CAPTION))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(p.danger)
                .child(rust_i18n::t!("ai.fresh_approval_badge").to_string()),
        );
    }

    if let Some(impact) = &approval.impact_note {
        card = card.child(
            div()
                .px(rems(space::XS))
                .py(px(4.0))
                .rounded(p.radius.sm)
                .bg(p.secondary)
                .border_1()
                .border_color(p.warning)
                .text_size(rems(typography::CAPTION))
                .text_color(p.warning)
                .child(rust_i18n::t!("ai.approval_impact", note = impact).to_string()),
        );
    }

    if let Some(diff) = &approval.diff_preview {
        card = card.child(
            div()
                .id("ai-approval-diff")
                .max_h(rems(12.0))
                .overflow_y_scroll()
                .child(crate::app::ai::diff_view::render_diff_block(diff, p)),
        );
    }

    let feedback_text = feedback_input.read(cx).value().trim().to_owned();
    let on_reject = on_decide.clone();
    let on_handoff = on_decide.clone();
    let on_allow_once = on_decide.clone();

    let mut actions_row = h_flex()
        .w_full()
        .flex_wrap()
        .justify_end()
        .gap(rems(space::XS))
        .child(
            Button::new("ai-approval-handoff")
                .ghost()
                .xsmall()
                .label(rust_i18n::t!("ai.approval_handoff").to_string())
                .on_click(move |_, w, cx| {
                    on_handoff(AiApprovalDecision::HandoffToTerminal, w, cx);
                }),
        )
        .child(
            Button::new("ai-approval-reject")
                .ghost()
                .small()
                .label(if feedback_text.is_empty() {
                    rust_i18n::t!("ai.reject").to_string()
                } else {
                    rust_i18n::t!("ai.reject_with_feedback").to_string()
                })
                .on_click(move |_, w, cx| {
                    on_reject(
                        AiApprovalDecision::RejectWithFeedback {
                            reason: feedback_text.clone(),
                        },
                        w,
                        cx,
                    );
                }),
        );

    if let Some(prefix) = &approval.suggested_prefix {
        let pfx = prefix.clone();
        let on_allow_pfx = on_decide;
        actions_row = actions_row.child(
            Button::new("ai-approval-allow-prefix")
                .outline()
                .small()
                .label(rust_i18n::t!("ai.approval_allow_prefix", prefix = pfx).to_string())
                .on_click(move |_, w, cx| {
                    on_allow_pfx(
                        AiApprovalDecision::AllowPrefix {
                            prefix: pfx.clone(),
                        },
                        w,
                        cx,
                    );
                }),
        );
    }

    actions_row = actions_row.child(
        Button::new("ai-approval-approve")
            .primary()
            .small()
            .label(rust_i18n::t!("ai.approve").to_string())
            .on_click(move |_, w, cx| on_allow_once(AiApprovalDecision::AllowOnce, w, cx)),
    );

    card = card
        .child(div().w_full().child(Input::new(feedback_input)))
        .child(actions_row);

    card.into_any_element()
}
