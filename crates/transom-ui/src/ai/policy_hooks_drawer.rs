//! 运行中发消息行为、命令前缀策略（Exec Policy）与生命周期钩子（Hooks）管理抽屉。

use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::*;
use transom_ai::{ExecPolicyAction, ExecPolicyRule, HookAction, HookEvent, HookMatcher, HookRule};
use transom_ui_icons::{AppIcon, AppIconElement, UiIcon};
use uuid::Uuid;

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

/// 渲染运行中发消息行为、命令策略与生命周期钩子管理块。
pub(super) fn render_policy_hooks_section(
    view: &AiAssistantView,
    p: &Palette,
    cx: &mut Context<AiAssistantView>,
) -> AnyElement {
    let rules = &view.state.settings.exec_policy_rules;
    let hooks = &view.state.settings.hooks;
    let cur_behavior = view.state.settings.running_message_behavior;

    let mut root = v_flex()
        .w_full()
        .gap(rems(space::SM))
        .child(super::queue_card::render_running_behavior_row(
            cur_behavior,
            p,
            cx,
        ))
        .child(
            v_flex()
                .w_full()
                .pt(rems(space::XS))
                .gap(rems(space::XS))
                .border_t_1()
                .border_color(p.divider(p.border))
                .child(
                    div()
                        .text_size(rems(typography::BODY))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(p.foreground)
                        .child(rust_i18n::t!("ai.exec_policy_title").to_string()),
                )
                .child(
                    h_flex()
                        .w_full()
                        .gap(rems(space::XS))
                        .items_center()
                        .child(div().flex_1().child(Input::new(&view.policy_pattern_input)))
                        .child(
                            Button::new("ai-add-policy-allow")
                                .outline()
                                .xsmall()
                                .label(rust_i18n::t!("ai.policy_add_allow").to_string())
                                .on_click(cx.listener(|this, _, w, cx| {
                                    this.add_exec_policy_rule(ExecPolicyAction::Allow, w, cx);
                                })),
                        )
                        .child(
                            Button::new("ai-add-policy-deny")
                                .outline()
                                .xsmall()
                                .label(rust_i18n::t!("ai.policy_add_deny").to_string())
                                .on_click(cx.listener(|this, _, w, cx| {
                                    this.add_exec_policy_rule(ExecPolicyAction::Deny, w, cx);
                                })),
                        ),
                ),
        );

    for (idx, rule) in rules.iter().enumerate() {
        let rid = rule.id.clone();
        let badge_color = match rule.action {
            ExecPolicyAction::Allow => p.success,
            ExecPolicyAction::Deny => p.danger,
            ExecPolicyAction::Ask => p.warning,
        };
        root = root.child(
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
                        .gap(px(6.0))
                        .items_center()
                        .child(
                            div()
                                .text_size(rems(typography::CAPTION))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(badge_color)
                                .child(rule.action.label()),
                        )
                        .child(
                            div()
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

    root = root.child(render_hooks_header_and_forms(view, p, cx));

    for (idx, hook) in hooks.iter().enumerate() {
        let hid = hook.id.clone();
        let hid_del = hook.id.clone();
        root = root.child(
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
                        .gap(px(6.0))
                        .items_center()
                        .overflow_hidden()
                        .child(
                            div()
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
                                .truncate()
                                .text_size(rems(typography::CAPTION))
                                .text_color(p.foreground)
                                .child(hook.name.clone()),
                        ),
                )
                .child(
                    h_flex()
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

    root.into_any_element()
}

fn render_hooks_header_and_forms(
    view: &AiAssistantView,
    p: &Palette,
    cx: &mut Context<AiAssistantView>,
) -> AnyElement {
    let ev_label = view.policy_hooks_form.selected_event.label();
    v_flex()
        .w_full()
        .pt(rems(space::XS))
        .gap(rems(space::XS))
        .border_t_1()
        .border_color(p.divider(p.border))
        .child(
            h_flex()
                .justify_between()
                .items_center()
                .child(
                    div()
                        .text_size(rems(typography::BODY))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(p.foreground)
                        .child(rust_i18n::t!("ai.hooks_title").to_string()),
                )
                .child(
                    Button::new("ai-add-nginx-hook")
                        .outline()
                        .xsmall()
                        .label(rust_i18n::t!("ai.hooks_add_preset").to_string())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.add_preset_nginx_hook(cx);
                        })),
                ),
        )
        .child(
            h_flex()
                .w_full()
                .gap(rems(space::XS))
                .items_center()
                .child(
                    Button::new("ai-hook-ev-cycle")
                        .outline()
                        .xsmall()
                        .label(ev_label)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.cycle_custom_hook_event(cx);
                        })),
                )
                .child(
                    div()
                        .w(rems(5.5))
                        .child(Input::new(&view.policy_hooks_form.tool_input)),
                )
                .child(
                    div()
                        .w(rems(5.5))
                        .child(Input::new(&view.policy_hooks_form.path_input)),
                )
                .child(
                    div()
                        .flex_1()
                        .child(Input::new(&view.policy_hooks_form.cmd_input)),
                )
                .child(
                    Button::new("ai-add-custom-hook")
                        .primary()
                        .xsmall()
                        .label(rust_i18n::t!("ai.hook_add_custom").to_string())
                        .on_click(cx.listener(|this, _, w, cx| {
                            this.add_custom_shell_hook(w, cx);
                        })),
                ),
        )
        .child(
            h_flex()
                .w_full()
                .gap(rems(space::XS))
                .items_center()
                .child(div().flex_1().child(Input::new(&view.hook_webhook_input)))
                .child(
                    Button::new("ai-add-webhook-hook")
                        .outline()
                        .xsmall()
                        .label(rust_i18n::t!("ai.hooks_add_webhook").to_string())
                        .on_click(cx.listener(|this, _, w, cx| {
                            this.add_webhook_stop_hook(w, cx);
                        })),
                ),
        )
        .into_any_element()
}

impl AiAssistantView {
    pub(super) fn cycle_custom_hook_event(&mut self, cx: &mut Context<Self>) {
        self.policy_hooks_form.selected_event = match self.policy_hooks_form.selected_event {
            HookEvent::PreToolUse => HookEvent::PostToolUse,
            HookEvent::PostToolUse => HookEvent::Stop,
            HookEvent::Stop => HookEvent::PreToolUse,
        };
        cx.notify();
    }

    pub(super) fn add_custom_shell_hook(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let cmd = self
            .policy_hooks_form
            .cmd_input
            .read(cx)
            .value()
            .trim()
            .to_owned();
        if cmd.is_empty() {
            return;
        }
        let tool = self
            .policy_hooks_form
            .tool_input
            .read(cx)
            .value()
            .trim()
            .to_owned();
        let path = self
            .policy_hooks_form
            .path_input
            .read(cx)
            .value()
            .trim()
            .to_owned();
        let ev = self.policy_hooks_form.selected_event;
        self.state.settings.hooks.push(HookRule {
            id: Uuid::new_v4().to_string(),
            name: format!("{}: {cmd}", ev.label()),
            event: ev,
            matcher: HookMatcher {
                tool_name: (!tool.is_empty()).then_some(tool),
                path_contains: (!path.is_empty()).then_some(path),
                command_prefix: None,
            },
            action: HookAction::ShellCommand {
                command: cmd,
                target: None,
            },
            enabled: true,
            block_on_failure: ev == HookEvent::PreToolUse,
        });
        self.policy_hooks_form
            .cmd_input
            .update(cx, |s, cx| s.set_value("", window, cx));
        let _ = self.ai_store.save_settings(&self.state.settings);
        cx.notify();
    }

    pub(super) fn add_exec_policy_rule(
        &mut self,
        action: ExecPolicyAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pattern = self.policy_pattern_input.read(cx).value().trim().to_owned();
        if pattern.is_empty() {
            return;
        }
        self.state.settings.exec_policy_rules.push(ExecPolicyRule {
            id: Uuid::new_v4().to_string(),
            pattern,
            action,
            note: String::new(),
            enabled: true,
        });
        self.policy_pattern_input
            .update(cx, |s, cx| s.set_value("", window, cx));
        let _ = self.ai_store.save_settings(&self.state.settings);
        cx.notify();
    }

    pub(super) fn delete_exec_policy_rule(&mut self, id: &str, cx: &mut Context<Self>) {
        self.state.settings.exec_policy_rules.retain(|r| r.id != id);
        let _ = self.ai_store.save_settings(&self.state.settings);
        cx.notify();
    }

    pub(super) fn add_preset_nginx_hook(&mut self, cx: &mut Context<Self>) {
        self.state.settings.hooks.push(HookRule {
            id: Uuid::new_v4().to_string(),
            name: rust_i18n::t!("ai.hook_preset_nginx_name").to_string(),
            event: HookEvent::PostToolUse,
            matcher: HookMatcher {
                tool_name: Some("remote_fs".to_owned()),
                path_contains: Some("nginx".to_owned()),
                command_prefix: None,
            },
            action: HookAction::ShellCommand {
                command: "nginx -t".to_owned(),
                target: None,
            },
            enabled: true,
            block_on_failure: false,
        });
        let _ = self.ai_store.save_settings(&self.state.settings);
        cx.notify();
    }

    pub(super) fn add_webhook_stop_hook(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let url = self.hook_webhook_input.read(cx).value().trim().to_owned();
        if url.is_empty() {
            return;
        }
        self.state.settings.hooks.push(HookRule {
            id: Uuid::new_v4().to_string(),
            name: rust_i18n::t!("ai.hook_webhook_name").to_string(),
            event: HookEvent::Stop,
            matcher: HookMatcher::default(),
            action: HookAction::Webhook {
                url,
                template: None,
            },
            enabled: true,
            block_on_failure: false,
        });
        self.hook_webhook_input
            .update(cx, |s, cx| s.set_value("", window, cx));
        let _ = self.ai_store.save_settings(&self.state.settings);
        cx.notify();
    }

    pub(super) fn toggle_hook_rule(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(rule) = self.state.settings.hooks.iter_mut().find(|r| r.id == id) {
            rule.enabled = !rule.enabled;
            let _ = self.ai_store.save_settings(&self.state.settings);
            cx.notify();
        }
    }

    pub(super) fn delete_hook_rule(&mut self, id: &str, cx: &mut Context<Self>) {
        self.state.settings.hooks.retain(|r| r.id != id);
        let _ = self.ai_store.save_settings(&self.state.settings);
        cx.notify();
    }
}
