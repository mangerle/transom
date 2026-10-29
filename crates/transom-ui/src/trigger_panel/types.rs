//! 终端触发器设置面板数据契约与表单状态模型。

use crate::app::AppShell;
use crate::session_form::text_input;
use gpui_kit::component::input::InputState;
use gpui_kit::*;
use transom_core::{TriggerAction, TriggerId, TriggerRule};

/// 触发器动作选项类型（表单展示用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ActionKindChoice {
    Bell,
    Notification,
    AutoResponse,
}

/// 触发器规则编辑表单状态。
#[derive(Clone)]
pub(crate) struct TriggerEditState {
    pub(crate) base_id: Option<TriggerId>,
    pub(crate) name: Entity<InputState>,
    pub(crate) pattern: Entity<InputState>,
    pub(crate) action_kind: ActionKindChoice,
    pub(crate) action_text: Entity<InputState>,
    pub(crate) case_sensitive: bool,
    pub(crate) error: Option<String>,
}

impl TriggerEditState {
    pub(crate) fn new(
        rule: Option<&TriggerRule>,
        window: &mut Window,
        cx: &mut Context<AppShell>,
    ) -> Self {
        if let Some(rule) = rule {
            let (action_kind, text) = match &rule.action {
                TriggerAction::Notification(t) => (ActionKindChoice::Notification, t.clone()),
                TriggerAction::AutoResponse(t) => (ActionKindChoice::AutoResponse, t.clone()),
                _ => (ActionKindChoice::Bell, String::new()),
            };
            let name_placeholder = rust_i18n::t!("trigger.name_placeholder");
            let pattern_placeholder = rust_i18n::t!("trigger.pattern_placeholder");
            let action_text_placeholder = rust_i18n::t!("trigger.action_text_placeholder");
            Self {
                base_id: Some(rule.id),
                name: text_input(window, cx, &rule.name, name_placeholder.as_ref()),
                pattern: text_input(window, cx, &rule.pattern, pattern_placeholder.as_ref()),
                action_kind,
                action_text: text_input(window, cx, &text, action_text_placeholder.as_ref()),
                case_sensitive: rule.case_sensitive,
                error: None,
            }
        } else {
            let name_placeholder = rust_i18n::t!("trigger.name_placeholder");
            let pattern_placeholder = rust_i18n::t!("trigger.pattern_placeholder");
            let action_text_placeholder = rust_i18n::t!("trigger.action_text_placeholder");
            Self {
                base_id: None,
                name: text_input(window, cx, "", name_placeholder.as_ref()),
                pattern: text_input(window, cx, "", pattern_placeholder.as_ref()),
                action_kind: ActionKindChoice::AutoResponse,
                action_text: text_input(window, cx, "", action_text_placeholder.as_ref()),
                case_sensitive: false,
                error: None,
            }
        }
    }
}
