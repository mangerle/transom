//! 终端触发器（Triggers）设置面板：规则清单、新建/编辑与删除。
//!
//! 设计原理（Why）：
//! - 实现初衷：为用户提供可视化的触发器规则维护面板，支持增删改查正则规则，配置响铃、桌面通知与自动应答动作；
//! - 核心优势：
//!   1. 统一集成在偏好设置“终端”标签页中，一目了然且支持单条启用/禁用；
//!   2. 编辑弹窗内置模式语法校验，防止非法正则表达式输入；
//!   3. 支持自动应答与通知文本动态捕获参数展开（`$0`, `$1` 等）；
//!   4. 保存时不仅持久化到配置文件，更实时下发至当前全部活跃终端窗格。

use crate::app::app_settings;
use crate::app::app_settings_mut;
use crate::app::update_app_settings;
use crate::palette::Palette;
use crate::settings::{SettingsEvent, SettingsView};
use gpui_kit::base::{Selectable, h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::switch::Switch;
use gpui_kit::*;
use regex::Regex;
use transom_core::{TriggerAction, TriggerId, TriggerRule};
use transom_ui_icons::AppIcon;
use transom_ui_icons::UiIcon;

mod types;

pub(crate) use types::{ActionKindChoice, TriggerEditState};

/// 获取动作说明文本。
pub(super) fn action_summary(action: &TriggerAction) -> String {
    match action {
        TriggerAction::Bell => rust_i18n::t!("trigger.action_bell").to_string(),
        TriggerAction::Notification(t) => {
            format!("{}: {t}", rust_i18n::t!("trigger.action_notification"))
        }
        TriggerAction::AutoResponse(t) => {
            format!("{}: {t}", rust_i18n::t!("trigger.action_auto_response"))
        }
        TriggerAction::Unknown => rust_i18n::t!("trigger.action_unknown").to_string(),
    }
}

impl SettingsView {
    /// 打开新建触发器规则弹窗。
    pub(crate) fn open_new_trigger(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let edit = TriggerEditState::new(None, window, cx);
        self.state.trigger_edit = Some(edit);
        cx.notify();
    }

    /// 打开编辑触发器规则弹窗。
    pub(crate) fn open_edit_trigger(
        &mut self,
        id: TriggerId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let rule = app_settings(cx).triggers.find_rule(id).cloned();
        if let Some(rule) = rule {
            let edit = TriggerEditState::new(Some(&rule), window, cx);
            self.state.trigger_edit = Some(edit);
            cx.notify();
        }
    }

    /// 提交保存触发器规则。
    pub(crate) fn save_trigger_edit(&mut self, cx: &mut Context<Self>) {
        let Some(edit) = self.state.trigger_edit.as_ref() else {
            return;
        };

        let name = edit.name.read(cx).value().trim().to_string();
        let pattern = edit.pattern.read(cx).value().trim().to_string();
        let action_text = edit.action_text.read(cx).value().trim().to_string();
        let action_kind = edit.action_kind;
        let case_sensitive = edit.case_sensitive;
        let base_id = edit.base_id;

        if pattern.is_empty() {
            self.set_trigger_edit_error(
                rust_i18n::t!("trigger.error_pattern_empty").to_string(),
                cx,
            );
            return;
        }

        // 校验正则表达式有效性
        let test_pattern = if case_sensitive {
            pattern.clone()
        } else {
            format!("(?i){pattern}")
        };
        if let Err(err) = Regex::new(&test_pattern) {
            self.set_trigger_edit_error(
                rust_i18n::t!("trigger.error_pattern_invalid", error = err.to_string()).to_string(),
                cx,
            );
            return;
        }

        let action = match action_kind {
            ActionKindChoice::Bell => TriggerAction::Bell,
            ActionKindChoice::Notification => TriggerAction::Notification(action_text),
            ActionKindChoice::AutoResponse => TriggerAction::AutoResponse(action_text),
        };

        let rule_id = base_id.unwrap_or_else(TriggerId::new);
        let rule = TriggerRule {
            id: rule_id,
            name: if name.is_empty() {
                pattern.clone()
            } else {
                name
            },
            enabled: true,
            pattern,
            action,
            case_sensitive,
        };

        if let Some(existing) = app_settings_mut(cx).triggers.find_rule_mut(rule_id) {
            *existing = rule;
        } else {
            app_settings_mut(cx).triggers.rules.push(rule);
        }

        self.state.trigger_edit = None;

        self.sync_triggers_to_live_panes(cx);
        cx.notify();
    }

    /// 切换编辑表单中的动作类型。
    fn set_trigger_action_kind(&mut self, kind: ActionKindChoice, cx: &mut Context<Self>) {
        if let Some(e) = self.state.trigger_edit.as_mut() {
            e.action_kind = kind;
        }
        cx.notify();
    }

    /// 设置编辑表单错误提示。
    fn set_trigger_edit_error(&mut self, err: String, cx: &mut Context<Self>) {
        if let Some(edit) = self.state.trigger_edit.as_mut() {
            edit.error = Some(err);
        }
        cx.notify();
    }

    /// 取消编辑触发器表单。
    fn cancel_trigger_edit(&mut self, cx: &mut Context<Self>) {
        self.state.trigger_edit = None;
        cx.notify();
    }

    /// 将触发器规则同步下发至当前所有活跃终端窗格。
    ///
    /// 终端实体归外壳工作区，弹窗只发意图；事件自带完整规则快照，
    /// 外壳处理时不需要回读设置实体。
    pub(crate) fn sync_triggers_to_live_panes(&mut self, cx: &mut Context<Self>) {
        cx.emit(SettingsEvent::ApplyTriggers(Box::new(
            app_settings(cx).triggers.clone(),
        )));
    }

    /// 切换单条规则启用状态。
    pub(crate) fn toggle_trigger_enabled(
        &mut self,
        id: TriggerId,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        if let Some(rule) = app_settings_mut(cx).triggers.find_rule_mut(id) {
            rule.enabled = enabled;
            self.sync_triggers_to_live_panes(cx);
            cx.notify();
        }
    }

    /// 删除指定触发器规则。
    pub(crate) fn delete_trigger_rule(&mut self, id: TriggerId, cx: &mut Context<Self>) {
        app_settings_mut(cx).triggers.remove_rule(id);
        if self
            .state
            .trigger_edit
            .as_ref()
            .is_some_and(|edit| edit.base_id == Some(id))
        {
            self.state.trigger_edit = None;
        }
        self.sync_triggers_to_live_panes(cx);
        cx.notify();
    }
}

/// 编辑器头部：标题、规则名与匹配模式字段。
pub(super) fn trigger_editor_fields(
    edit_state: &crate::trigger_panel::TriggerEditState,
    p: &Palette,
) -> AnyElement {
    let is_new = edit_state.base_id.is_none();
    v_flex()
        .gap_2p5()
        .child(
            div().text_sm().font_weight(FontWeight::BOLD).child(
                rust_i18n::t!(if is_new {
                    "trigger.new_rule_title"
                } else {
                    "trigger.edit_rule_title"
                })
                .to_string(),
            ),
        )
        .child(crate::widgets::form_field(
            rust_i18n::t!("trigger.rule_name").to_string(),
            edit_state.name.clone(),
            p,
        ))
        .child(crate::widgets::form_field(
            rust_i18n::t!("trigger.rule_pattern").to_string(),
            edit_state.pattern.clone(),
            p,
        ))
        .into_any_element()
}

/// 单条触发器行：启用开关、名称/动作摘要与编辑/删除入口。
pub(super) fn trigger_rule_row(
    rule: &transom_core::TriggerRule,
    p: &Palette,
    cx: &mut Context<SettingsView>,
) -> AnyElement {
    let id = rule.id;
    let is_enabled = rule.enabled;
    let action_desc = action_summary(&rule.action);
    let row = h_flex()
        .id(ElementId::Name(format!("trigger-row-{id}").into()))
        .px_2()
        .py_1p5()
        .gap_2()
        .items_center()
        .border_b_1()
        .border_color(p.stroke.strong)
        .child(
            Switch::new(ElementId::Name(format!("trigger-switch-{id}").into()))
                .checked(is_enabled)
                .on_click(cx.listener(move |this, &checked, _, cx| {
                    this.toggle_trigger_enabled(id, checked, cx);
                })),
        )
        .child(
            v_flex()
                .flex_1()
                .gap_0p5()
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .child(rule.name.clone()),
                        )
                        .child(
                            div()
                                .text_xs()
                                .px_1p5()
                                .rounded_sm()
                                .bg(p.muted)
                                .text_color(p.muted_foreground)
                                .child(rule.pattern.clone()),
                        ),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(action_desc),
                ),
        )
        .child(
            Button::new(ElementId::Name(format!("trigger-edit-{id}").into()))
                .ghost()
                .xsmall()
                .label(rust_i18n::t!("common.edit").to_string())
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.open_edit_trigger(id, window, cx);
                })),
        )
        .child(
            crate::widgets::IconButton::new(
                ElementId::Name(format!("trigger-delete-{id}").into()),
                AppIcon::Ui(UiIcon::DELETE),
                rust_i18n::t!("common.delete").to_string(),
            )
            .size(crate::widgets::IconButtonSize::Compact)
            .render()
            .on_click(cx.listener(move |this, _, _, cx| {
                this.delete_trigger_rule(id, cx);
            })),
        );
    row.into_any_element()
}

mod view;
