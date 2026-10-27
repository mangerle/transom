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
            Self {
                base_id: Some(rule.id),
                name: text_input(window, cx, &rule.name, "如：监控错误告警或自动输入密码"),
                pattern: text_input(window, cx, &rule.pattern, "正则表达式，如：(?i)password:"),
                action_kind,
                action_text: text_input(window, cx, &text, "应答内容或通知文本"),
                case_sensitive: rule.case_sensitive,
                error: None,
            }
        } else {
            Self {
                base_id: None,
                name: text_input(window, cx, "", "如：监控错误告警或自动输入密码"),
                pattern: text_input(window, cx, "", "正则表达式，如：(?i)password:"),
                action_kind: ActionKindChoice::AutoResponse,
                action_text: text_input(window, cx, "", "应答内容或通知文本"),
                case_sensitive: false,
                error: None,
            }
        }
    }
}
