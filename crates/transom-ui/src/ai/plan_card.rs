//! AI 实时任务清单卡片与底部实时活动状态行。

use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::progress::ProgressCircle;
use gpui_kit::*;
use transom_core::{AiPlanDto, AiPlanStepStatus};
use transom_ui_icons::{AppIcon, AppIconElement, UiIcon};

use crate::palette::Palette;
use crate::theme::tokens::{space, typography};

/// 实时任务清单卡片渲染参数。
pub(super) struct PlanCardParams<'a> {
    pub(super) plan: Option<&'a AiPlanDto>,
    pub(super) focus: Option<&'a str>,
    pub(super) elapsed_secs: Option<u64>,
    pub(super) plan_mode: bool,
    pub(super) is_running: bool,
    pub(super) expanded: bool,
    pub(super) p: &'a Palette,
}

/// 渲染实时原地更新的任务清单卡片（支持运行中/完成后随时折叠或展开）与当前活动状态行。
pub(super) fn render_plan_card(
    params: PlanCardParams<'_>,
    on_toggle_expand: impl Fn(&mut Window, &mut App) + 'static,
    on_execute_plan: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    let PlanCardParams {
        plan,
        focus,
        elapsed_secs,
        plan_mode,
        is_running,
        expanded,
        p,
    } = params;
    if !expanded {
        return div().into_any_element();
    }
    let mut container = v_flex().w_full().gap(rems(space::XS));

    let dur_label = match elapsed_secs {
        Some(secs) if secs >= 60 => format!("{}m {}s", secs / 60, secs % 60),
        Some(secs) if secs > 0 => format!("{secs}s"),
        _ => String::new(),
    };
    let status_text = focus
        .or_else(|| plan.and_then(|pl| pl.focus.as_deref()))
        .map(str::trim)
        .filter(|s| !s.is_empty());

    if let Some(plan) = plan
        && !plan.steps.is_empty()
    {
        let done_count = plan
            .steps
            .iter()
            .filter(|s| s.status == AiPlanStepStatus::Done)
            .count();
        let total = plan.steps.len();
        let ratio = (done_count as f32 / total as f32).clamp(0.0, 1.0);
        let heading = plan
            .title
            .as_deref()
            .filter(|t| !t.trim().is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| rust_i18n::t!("ai.plan_title").to_string());

        let mut card = v_flex()
            .w_full()
            .p(rems(space::SM))
            .gap(rems(space::XS))
            .rounded(p.radius.md)
            .bg(p.surface.floating)
            .shadow_xs()
            .border_1()
            .border_color(p.divider(p.border))
            .child(
                h_flex()
                    .id("ai-plan-header")
                    .justify_between()
                    .items_center()
                    .cursor_pointer()
                    .on_click(move |_, w, cx| on_toggle_expand(w, cx))
                    .child(
                        div()
                            .text_size(rems(typography::CAPTION))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(p.foreground)
                            .child(heading),
                    )
                    .child(
                        h_flex()
                            .gap(px(6.0))
                            .items_center()
                            .child(
                                div()
                                    .px(px(6.0))
                                    .py(px(1.0))
                                    .rounded_full()
                                    .bg(p.secondary)
                                    .text_size(rems(typography::CAPTION))
                                    .text_color(p.text.muted)
                                    .child(format!("{done_count}/{total}")),
                            )
                            .child(div().text_color(p.text.muted).child(
                                AppIconElement::new(AppIcon::Ui(UiIcon::CHEVRON_DOWN)).small(),
                            )),
                    ),
            )
            .child(
                div()
                    .w_full()
                    .h(px(3.0))
                    .rounded_full()
                    .bg(p.secondary)
                    .overflow_hidden()
                    .child(div().h_full().w(relative(ratio)).bg(p.primary)),
            );

        for step in &plan.steps {
            let (icon, icon_color, text_color, weight) = match step.status {
                AiPlanStepStatus::Done => {
                    (UiIcon::CHECK, p.success, p.text.muted, FontWeight::NORMAL)
                }
                AiPlanStepStatus::InProgress => (
                    UiIcon::REFRESH,
                    p.primary,
                    p.foreground,
                    FontWeight::SEMIBOLD,
                ),
                AiPlanStepStatus::Failed => (UiIcon::ERROR, p.danger, p.danger, FontWeight::NORMAL),
                AiPlanStepStatus::Pending => (
                    UiIcon::PAUSE,
                    p.text.muted,
                    p.muted_foreground,
                    FontWeight::NORMAL,
                ),
            };
            card = card.child(
                h_flex()
                    .w_full()
                    .gap(rems(space::XS))
                    .items_center()
                    .child(
                        div()
                            .text_color(icon_color)
                            .child(AppIconElement::new(AppIcon::Ui(icon)).small()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_size(rems(typography::CAPTION))
                            .font_weight(weight)
                            .text_color(text_color)
                            .child(step.text.clone()),
                    ),
            );
        }
        if plan_mode && !is_running {
            card = card.child(
                h_flex().w_full().justify_end().pt(px(2.0)).child(
                    Button::new("ai-plan-start-exec")
                        .primary()
                        .xsmall()
                        .label(rust_i18n::t!("ai.plan_start_execute").to_string())
                        .on_click(move |_, w, cx| on_execute_plan(w, cx)),
                ),
            );
        }
        container = container.child(card);
    }

    if let Some(text) = status_text {
        let mut row = h_flex()
            .w_full()
            .px(rems(space::XS))
            .justify_between()
            .items_center()
            .child(
                h_flex()
                    .flex_1()
                    .gap(px(6.0))
                    .items_center()
                    .overflow_hidden()
                    .child(div().w(px(6.0)).h(px(6.0)).rounded_full().bg(p.primary))
                    .child(
                        div()
                            .flex_1()
                            .truncate()
                            .text_size(rems(typography::CAPTION))
                            .text_color(p.text.muted)
                            .child(text.to_owned()),
                    ),
            );
        if !dur_label.is_empty() {
            row = row.child(
                div()
                    .text_size(rems(typography::CAPTION))
                    .text_color(p.text.muted)
                    .child(dur_label),
            );
        }
        container = container.child(row);
    }

    container.into_any_element()
}

/// 渲染消息流底部居中的“滚动到最底部”悬浮圆形按钮。
pub(super) fn render_scroll_bottom_button(
    p: &Palette,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> AnyElement {
    let tip = rust_i18n::t!("ai.scroll_to_bottom").to_string();
    h_flex()
        .absolute()
        .bottom(rems(space::SM))
        .left_0()
        .right_0()
        .justify_center()
        .child(
            h_flex()
                .id("ai-scroll-bottom-btn")
                .w(px(30.0))
                .h(px(30.0))
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(p.surface.floating)
                .border_1()
                .border_color(p.border)
                .shadow_md()
                .cursor_pointer()
                .hover(|s| s.bg(p.surface.hover))
                .tooltip(move |w, cx| {
                    gpui_kit::component::tooltip::Tooltip::new(tip.clone()).build(w, cx)
                })
                .on_click(move |_, w, cx| on_click(w, cx))
                .child(
                    div()
                        .text_color(p.foreground)
                        .child(AppIconElement::new(AppIcon::Ui(UiIcon::DOWN)).small()),
                ),
        )
        .into_any_element()
}

/// 输入栏上方并排极简胶囊条的渲染参数。
pub(super) struct StatusPillsParams<'a> {
    pub(super) plan: Option<&'a AiPlanDto>,
    pub(super) focus: Option<&'a str>,
    pub(super) plan_expanded: bool,
    pub(super) running_tasks_count: usize,
    pub(super) tasks_expanded: bool,
    pub(super) p: &'a Palette,
}

/// 在输入栏上方渲染并排的步骤胶囊与运行中命令胶囊（无内容时返回 `None`）。
pub(super) fn render_status_pills_row(
    params: StatusPillsParams<'_>,
    on_toggle_plan: impl Fn(&mut Window, &mut App) + 'static,
    on_toggle_tasks: impl Fn(&mut Window, &mut App) + 'static,
) -> Option<AnyElement> {
    let StatusPillsParams {
        plan,
        focus,
        plan_expanded,
        running_tasks_count,
        tasks_expanded,
        p,
    } = params;
    let has_plan_pill =
        plan.is_some_and(|pl| !pl.steps.is_empty()) || focus.is_some_and(|f| !f.trim().is_empty());
    let has_task_pill = running_tasks_count > 0;
    if !has_plan_pill && !has_task_pill {
        return None;
    }

    let mut row = h_flex()
        .w_full()
        .px(rems(space::SM))
        .pb(rems(space::XS))
        .gap(rems(space::XS))
        .items_center()
        .flex_wrap();

    if has_plan_pill {
        let (done, total, pct) = match plan.filter(|pl| !pl.steps.is_empty()) {
            Some(pl) => {
                let d = pl
                    .steps
                    .iter()
                    .filter(|s| s.status == AiPlanStepStatus::Done)
                    .count();
                let t = pl.steps.len();
                (d, t, ((d as f32 / t as f32) * 100.0).clamp(0.0, 100.0))
            }
            None => (0, 0, 15.0),
        };
        let label = if total > 0 {
            rust_i18n::t!("ai.work_steps_summary", done = done, total = total).to_string()
        } else {
            focus
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| rust_i18n::t!("ai.plan_title").to_string())
        };
        let mut pill = h_flex()
            .id("ai-plan-pill")
            .px(px(10.0))
            .py(px(4.0))
            .gap(px(6.0))
            .items_center()
            .rounded(p.radius.md)
            .bg(p.secondary)
            .border_1()
            .border_color(if plan_expanded {
                p.primary
            } else {
                p.divider(p.border)
            })
            .cursor_pointer()
            .hover(|s| s.bg(p.surface.hover))
            .on_click(move |_, w, cx| on_toggle_plan(w, cx))
            .child(
                ProgressCircle::new("ai-plan-pill-ring")
                    .value(pct)
                    .size(px(14.0))
                    .color(if total > 0 && done == total {
                        p.success
                    } else {
                        p.primary
                    }),
            )
            .child(
                div()
                    .max_w(px(180.0))
                    .truncate()
                    .text_size(rems(typography::CAPTION))
                    .text_color(if plan_expanded {
                        p.foreground
                    } else {
                        p.text.muted
                    })
                    .child(label),
            );
        pill = pill.child(
            div().text_color(p.text.muted).child(
                AppIconElement::new(AppIcon::Ui(if plan_expanded {
                    UiIcon::CHEVRON_DOWN
                } else {
                    UiIcon::CHEVRON_RIGHT
                }))
                .small(),
            ),
        );
        row = row.child(pill);
    }

    if has_task_pill {
        let label =
            rust_i18n::t!("ai.tasks_running_summary", count = running_tasks_count).to_string();
        let pill = h_flex()
            .id("ai-tasks-pill")
            .px(px(10.0))
            .py(px(4.0))
            .gap(px(6.0))
            .items_center()
            .rounded(p.radius.md)
            .bg(p.secondary)
            .border_1()
            .border_color(if tasks_expanded {
                p.primary
            } else {
                p.divider(p.border)
            })
            .cursor_pointer()
            .hover(|s| s.bg(p.surface.hover))
            .on_click(move |_, w, cx| on_toggle_tasks(w, cx))
            .child(
                div()
                    .text_color(p.primary)
                    .child(AppIconElement::new(AppIcon::Ui(UiIcon::REFRESH)).small()),
            )
            .child(
                div()
                    .text_size(rems(typography::CAPTION))
                    .text_color(if tasks_expanded {
                        p.foreground
                    } else {
                        p.text.muted
                    })
                    .child(label),
            )
            .child(
                div().text_color(p.text.muted).child(
                    AppIconElement::new(AppIcon::Ui(if tasks_expanded {
                        UiIcon::CHEVRON_DOWN
                    } else {
                        UiIcon::CHEVRON_RIGHT
                    }))
                    .small(),
                ),
            );
        row = row.child(pill);
    }

    Some(row.into_any_element())
}
