//! AI 面板命令策略规则（Exec Policy）与生命周期钩子（Hooks）交互操作逻辑。

use gpui_kit::*;
use transom_ai_contracts::{
    ExecPolicyAction, ExecPolicyRule, HookAction, HookEvent, HookMatcher, HookRule,
};
use uuid::Uuid;

use super::view::AiAssistantView;

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
        self.state.config.settings.hooks.push(HookRule {
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
        self.persist_settings_reported(cx);
    }

    pub(super) fn add_exec_policy_rule(
        &mut self,
        action: ExecPolicyAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pattern = self
            .policy_hooks_form
            .policy_pattern_input
            .read(cx)
            .value()
            .trim()
            .to_owned();
        if pattern.is_empty() {
            return;
        }
        self.state
            .config
            .settings
            .exec_policy_rules
            .push(ExecPolicyRule {
                id: Uuid::new_v4().to_string(),
                pattern,
                action,
                note: String::new(),
                enabled: true,
            });
        self.policy_hooks_form
            .policy_pattern_input
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.persist_settings_reported(cx);
    }

    pub(super) fn delete_exec_policy_rule(&mut self, id: &str, cx: &mut Context<Self>) {
        self.state
            .config
            .settings
            .exec_policy_rules
            .retain(|r| r.id != id);
        self.persist_settings_reported(cx);
    }

    pub(super) fn add_webhook_stop_hook(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let url = self
            .policy_hooks_form
            .hook_webhook_input
            .read(cx)
            .value()
            .trim()
            .to_owned();
        if url.is_empty() {
            return;
        }
        self.state.config.settings.hooks.push(HookRule {
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
        self.policy_hooks_form
            .hook_webhook_input
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.persist_settings_reported(cx);
    }

    pub(super) fn toggle_hook_rule(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(rule) = self
            .state
            .config
            .settings
            .hooks
            .iter_mut()
            .find(|r| r.id == id)
        {
            rule.enabled = !rule.enabled;
            self.persist_settings_reported(cx);
        }
    }

    pub(super) fn delete_hook_rule(&mut self, id: &str, cx: &mut Context<Self>) {
        self.state.config.settings.hooks.retain(|r| r.id != id);
        self.persist_settings_reported(cx);
    }
}
