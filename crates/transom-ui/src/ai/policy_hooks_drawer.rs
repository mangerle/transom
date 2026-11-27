//! 运行中发消息行为、命令前缀策略（Exec Policy）与生命周期钩子（Hooks）管理抽屉。

use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::*;
use transom_ai::{ExecPolicyAction, HookEvent};
use transom_ui_icons::{AppIcon, AppIconElement, UiIcon};

use super::config_drawer::{field_label, section_card};
use super::view::AiAssistantView;
use crate::palette::Palette;
use crate::theme::tokens::{space, typography};

/// 自定义 Hook 创建表单状态。
pub(super) struct PolicyHooksFormState {
    pub(super) selected_event: HookEvent,
    pub(super) tool_input: Entity<InputState>,
    pub(super) path_input: Entity<InputState>,
    pub(super) cmd_input: Entity<InputState>,
}

impl PolicyHooksFormState {
    pub(super) fn new(window: &mut Window, cx: &mut App) -> Self {
        let tool_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("ai.hook_tool_placeholder").to_string())
        });
        let path_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("ai.hook_path_placeholder").to_string())
        });
        let cmd_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("ai.hook_cmd_placeholder").to_string())
        });
        Self {
            selected_event: HookEvent::PostToolUse,
            tool_input,
            path_input,
            cmd_input,
        }
    }
}

/// 渲染命令策略与生命周期钩子管理块（卡片化布局，适配窄侧栏宽度）。
pub(super) fn render_policy_hooks_section(
    view: &AiAssistantView,
    p: &Palette,
    cx: &mut Context<AiAssistantView>,
) -> AnyElement {
    let rules = &view.state.settings.exec_policy_rules;
    let hooks = &view.state.settings.hooks;

    let mut policy_card = section_card(
        rust_i18n::t!("ai.exec_policy_title").to_string(),
        None,
        p,
    )
    .child(field_label(
        "按命令前缀快速放行或拦截 AI 的远端命令调用".to_owned(),
        p,
    ))
    .child(
        h_flex()
            .w_full()
            .gap(rems(space::XS))
            .items_center()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(Input::new(&view.policy_pattern_input).small()),
            )
            .child(
                Button::new("ai-add-policy-allow")
                    .outline()
                    .small()
                    .child(
                        h_flex()
                            .gap(px(4.0))
                            .items_center()
                            .child(
                                div()
                                    .text_color(p.success)
                                    .child(AppIconElement::new(AppIcon::Ui(UiIcon::CHECK)).small()),
                            )
                            .child("放行"),
                    )
                    .on_click(cx.listener(|this, _, w, cx| {
                        this.add_exec_policy_rule(ExecPolicyAction::Allow, w, cx);
                    })),
            )
            .child(
                Button::new("ai-add-policy-deny")
                    .outline()
                    .small()
                    .child(
                        h_flex()
                            .gap(px(4.0))
                            .items_center()
                            .child(
                                div()
                                    .text_color(p.danger)
                                    .child(AppIconElement::new(AppIcon::Ui(UiIcon::CLOSE)).small()),
                            )
                            .child("拦截"),
                    )
                    .on_click(cx.listener(|this, _, w, cx| {
                        this.add_exec_policy_rule(ExecPolicyAction::Deny, w, cx);
                    })),
            ),
    );

    if rules.is_empty() {
        policy_card = policy_card.child(
            div()
                .w_full()
                .px(rems(space::SM))
                .py(rems(space::XS))
                .rounded(p.radius.sm)
                .bg(p.secondary)
                .text_size(rems(typography::CAPTION))
                .text_color(p.text.muted)
                .child("暂无自定义前缀规则，当前遵循执行模式默认策略。"),
        );
    }

    for (idx, rule) in rules.iter().enumerate() {
        let rid = rule.id.clone();
        let badge_color = match rule.action {
            ExecPolicyAction::Allow => p.success,
            ExecPolicyAction::Deny => p.danger,
            ExecPolicyAction::Ask => p.warning,
        };
        policy_card = policy_card.child(
            h_flex()
                .id(("exec-policy-row", idx))
                .w_full()
                .px(rems(space::XS))
                .py(px(4.0))
                .justify_between()
                .items_center()
                .rounded(p.radius.sm)
                .bg(p.secondary)
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
                                .px(px(6.0))
                                .py(px(1.0))
                                .rounded(p.radius.sm)
                                .bg(badge_color.opacity(0.12))
                                .text_size(rems(typography::CAPTION))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(badge_color)
                                .child(rule.action.label()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .font_family("monospace")
                                .text_size(rems(typography::CAPTION))
                                .text_color(p.foreground)
                                .child(rule.pattern.clone()),
                        ),
                )
                .child(
                    Button::new(("del-policy", idx))
                        .ghost()
                        .xsmall()
                        .child(AppIconElement::new(AppIcon::Ui(UiIcon::CLOSE)).small())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.delete_exec_policy_rule(&rid, cx);
                        })),
                ),
        );
    }

    let mut hooks_card = render_hooks_card(view, p, cx);
    for (idx, hook) in hooks.iter().enumerate() {
        let hid = hook.id.clone();
        let hid_del = hook.id.clone();
        hooks_card = hooks_card.child(
            h_flex()
                .id(("ai-hook-row", idx))
                .w_full()
                .px(rems(space::XS))
                .py(px(4.0))
                .justify_between()
                .items_center()
                .rounded(p.radius.sm)
                .bg(p.secondary)
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
                                .px(px(6.0))
                                .py(px(1.0))
                                .rounded(p.radius.sm)
                                .bg(p.surface.floating)
                                .text_size(rems(typography::CAPTION))
                                .text_color(if hook.enabled {
                                    p.primary
                                } else {
                                    p.text.muted
                                })
                                .child(hook.event.label()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(rems(typography::CAPTION))
                                .text_color(p.foreground)
                                .child(hook.name.clone()),
                        ),
                )
                .child(
                    h_flex()
                        .flex_shrink_0()
                        .gap(px(4.0))
                        .child(
                            Button::new(("toggle-hook", idx))
                                .ghost()
                                .xsmall()
                                .label(if hook.enabled {
                                    rust_i18n::t!("ai.enabled").to_string()
                                } else {
                                    rust_i18n::t!("ai.disabled").to_string()
                                })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.toggle_hook_rule(&hid, cx);
                                })),
                        )
                        .child(
                            Button::new(("del-hook", idx))
                                .ghost()
                                .xsmall()
                                .child(AppIconElement::new(AppIcon::Ui(UiIcon::CLOSE)).small())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.delete_hook_rule(&hid_del, cx);
                                })),
                        ),
                ),
        );
    }

    v_flex()
        .w_full()
        .gap(rems(space::SM))
        .child(policy_card)
        .child(hooks_card)
        .into_any_element()
}

fn render_hooks_card(
    view: &AiAssistantView,
    p: &Palette,
    cx: &mut Context<AiAssistantView>,
) -> Div {
    let ev_label = view.policy_hooks_form.selected_event.label();
    let preset_btn = Button::new("ai-add-nginx-hook")
        .ghost()
        .small()
        .label("Nginx 校验预设")
        .on_click(cx.listener(|this, _, _, cx| {
            this.add_preset_nginx_hook(cx);
        }))
        .into_any_element();

    section_card(
        rust_i18n::t!("ai.hooks_title").to_string(),
        Some(preset_btn),
        p,
    )
    .child(
        v_flex()
            .w_full()
            .gap(rems(space::XS))
            .child(field_label("Shell 校验钩子".to_owned(), p))
            .child(
                h_flex()
                    .w_full()
                    .gap(rems(space::XS))
                    .items_center()
                    .child(
                        Button::new("ai-hook-ev-cycle")
                            .outline()
                            .small()
                            .child(
                                h_flex()
                                    .gap(px(4.0))
                                    .items_center()
                                    .child(ev_label)
                                    .child(AppIconElement::new(AppIcon::Ui(UiIcon::CHEVRON_DOWN)).small()),
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.cycle_custom_hook_event(cx);
                            })),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(Input::new(&view.policy_hooks_form.tool_input).small()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(Input::new(&view.policy_hooks_form.path_input).small()),
                    ),
            )
            .child(
                h_flex()
                    .w_full()
                    .gap(rems(space::XS))
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(Input::new(&view.policy_hooks_form.cmd_input).small()),
                    )
                    .child(
                        Button::new("ai-add-custom-hook")
                            .primary()
                            .small()
                            .child(
                                h_flex()
                                    .gap(px(4.0))
                                    .items_center()
                                    .child(AppIconElement::new(AppIcon::Ui(UiIcon::ADD)).small())
                                    .child("添加钩子"),
                            )
                            .on_click(cx.listener(|this, _, w, cx| {
                                this.add_custom_shell_hook(w, cx);
                            })),
                    ),
            ),
    )
    .child(
        v_flex()
            .w_full()
            .pt(rems(space::XS))
            .gap(rems(space::XS))
            .border_t_1()
            .border_color(p.divider(p.border))
            .child(field_label("Webhook 结束通知".to_owned(), p))
            .child(
                h_flex()
                    .w_full()
                    .gap(rems(space::XS))
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(Input::new(&view.hook_webhook_input).small()),
                    )
                    .child(
                        Button::new("ai-add-webhook-hook")
                            .outline()
                            .small()
                            .child(
                                h_flex()
                                    .gap(px(4.0))
                                    .items_center()
                                    .child(AppIconElement::new(AppIcon::Ui(UiIcon::ADD)).small())
                                    .child("添加通知"),
                            )
                            .on_click(cx.listener(|this, _, w, cx| {
                                this.add_webhook_stop_hook(w, cx);
                            })),
                    ),
            ),
    )
}
