//! 终端触发器（Triggers）设置面板：规则清单、新建/编辑与删除。
//!
//! 设计原理（Why）：
//! - 实现初衷：为用户提供可视化的触发器规则维护面板，支持增删改查正则规则，配置响铃、桌面通知与自动应答动作；
//! - 核心优势：
//!   1. 统一集成在偏好设置“终端”标签页中，一目了然且支持单条启用/禁用；
//!   2. 编辑弹窗内置模式语法校验，防止非法正则表达式输入；
//!   3. 支持自动应答与通知文本动态捕获参数展开（`$0`, `$1` 等）；
//!   4. 保存时不仅持久化到配置文件，更实时下发至当前全部活跃终端窗格。

use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{Selectable, h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::switch::Switch;
use gpui_kit::*;
use regex::Regex;
use transom_core::{TriggerAction, TriggerId, TriggerRule};

mod types;

pub(crate) use types::{ActionKindChoice, TriggerEditState};

/// 获取动作说明文本。
fn action_summary(action: &TriggerAction) -> String {
    match action {
        TriggerAction::Bell => rust_i18n::t!("trigger.action_bell").to_string(),
        TriggerAction::Notification(t) => {
            format!("{}: {t}", rust_i18n::t!("trigger.action_notification"))
        }
        TriggerAction::AutoResponse(t) => {
            format!("{}: {t}", rust_i18n::t!("trigger.action_auto_response"))
        }
        TriggerAction::Unknown => "未知动作".to_string(),
    }
}

impl AppShell {
    /// 打开新建触发器规则弹窗。
    pub(crate) fn open_new_trigger(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.modals.settings.trigger_edit = Some(TriggerEditState::new(None, window, cx));
        cx.notify();
    }

    /// 打开编辑触发器规则弹窗。
    pub(crate) fn open_edit_trigger(
        &mut self,
        id: TriggerId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(rule) = self.settings.triggers.find_rule(id) {
            self.modals.settings.trigger_edit = Some(TriggerEditState::new(Some(rule), window, cx));
            cx.notify();
        }
    }

    /// 提交保存触发器规则。
    pub(crate) fn save_trigger_edit(&mut self, cx: &mut Context<Self>) {
        let Some(edit) = self.modals.settings.trigger_edit.as_ref() else {
            return;
        };

        let name = edit.name.read(cx).value().trim().to_string();
        let pattern = edit.pattern.read(cx).value().trim().to_string();
        let action_text = edit.action_text.read(cx).value().trim().to_string();
        let action_kind = edit.action_kind;
        let case_sensitive = edit.case_sensitive;
        let base_id = edit.base_id;

        if pattern.is_empty() {
            self.set_trigger_edit_error("正则表达式模式串不能为空".to_string(), cx);
            return;
        }

        // 校验正则表达式有效性
        let test_pattern = if case_sensitive {
            pattern.clone()
        } else {
            format!("(?i){pattern}")
        };
        if let Err(err) = Regex::new(&test_pattern) {
            self.set_trigger_edit_error(format!("正则表达式语法错误: {err}"), cx);
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

        if let Some(existing) = self.settings.triggers.find_rule_mut(rule_id) {
            *existing = rule;
        } else {
            self.settings.triggers.rules.push(rule);
        }

        self.modals.settings.trigger_edit = None;

        self.sync_triggers_to_live_panes(cx);
        cx.notify();
    }

    /// 切换编辑表单中的动作类型。
    fn set_trigger_action_kind(&mut self, kind: ActionKindChoice, cx: &mut Context<Self>) {
        if let Some(e) = self.modals.settings.trigger_edit.as_mut() {
            e.action_kind = kind;
        }
        cx.notify();
    }

    /// 设置编辑表单错误提示。
    fn set_trigger_edit_error(&mut self, err: String, cx: &mut Context<Self>) {
        if let Some(edit) = self.modals.settings.trigger_edit.as_mut() {
            edit.error = Some(err);
        }
        cx.notify();
    }

    /// 取消编辑触发器表单。
    fn cancel_trigger_edit(&mut self, cx: &mut Context<Self>) {
        self.modals.settings.trigger_edit = None;
        cx.notify();
    }

    /// 将触发器规则同步下发至当前所有活跃终端窗格。
    pub(crate) fn sync_triggers_to_live_panes(&self, cx: &mut Context<Self>) {
        let triggers = self.settings.triggers.clone();
        for tab in &self.workspace.tabs {
            if let Some(term) = tab.terminal() {
                term.update(cx, |view, _| view.reload_triggers(&triggers));
            }
        }
    }

    /// 切换单条规则启用状态。
    pub(crate) fn toggle_trigger_enabled(
        &mut self,
        id: TriggerId,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        if let Some(rule) = self.settings.triggers.find_rule_mut(id) {
            rule.enabled = enabled;
            self.sync_triggers_to_live_panes(cx);
            cx.notify();
        }
    }

    /// 删除指定触发器规则。
    pub(crate) fn delete_trigger_rule(&mut self, id: TriggerId, cx: &mut Context<Self>) {
        self.settings.triggers.remove_rule(id);
        if self
            .modals
            .settings
            .trigger_edit
            .as_ref()
            .is_some_and(|edit| edit.base_id == Some(id))
        {
            self.modals.settings.trigger_edit = None;
        }
        self.sync_triggers_to_live_panes(cx);
        cx.notify();
    }

    /// 渲染偏好设置中的触发器配置组。
    pub(crate) fn render_settings_triggers(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let enabled = self.settings.triggers.enabled;
        let rules = self.settings.triggers.rules.clone();
        let edit = self.modals.settings.trigger_edit.clone();

        let mut list = v_flex().gap_1();
        if rules.is_empty() {
            list = list.child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("trigger.empty").to_string()),
            );
        }

        for rule in rules {
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
                .border_color(p.border.opacity(0.5))
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
                    Button::new(ElementId::Name(format!("trigger-delete-{id}").into()))
                        .ghost()
                        .xsmall()
                        .icon(IconName::Trash)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.delete_trigger_rule(id, cx);
                        })),
                );
            list = list.child(row);
        }

        let mut body = v_flex()
            .gap_3()
            .child(Self::setting_row(
                "trigger.master_switch",
                Some("trigger.master_switch_desc"),
                Switch::new("toggle-triggers-master")
                    .checked(enabled)
                    .on_click(cx.listener(|this, &checked, _, cx| {
                        this.settings.triggers.enabled = checked;
                        this.sync_triggers_to_live_panes(cx);
                        cx.notify();
                    }))
                    .into_any_element(),
                p,
            ))
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::BOLD)
                            .child(rust_i18n::t!("trigger.rules_list").to_string()),
                    )
                    .child(
                        Button::new("add-trigger-btn")
                            .small()
                            .icon(IconName::Plus)
                            .label(rust_i18n::t!("trigger.add_rule").to_string())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_new_trigger(window, cx);
                            })),
                    ),
            )
            .child(list);

        // 如果处于编辑状态，渲染内联编辑表单
        if let Some(edit_state) = edit {
            let is_new = edit_state.base_id.is_none();
            let action_kind = edit_state.action_kind;

            let mut form = v_flex()
                .p_3()
                .rounded_lg()
                .border_1()
                .border_color(p.border)
                .bg(p.popover)
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
                .child(Self::form_field(
                    rust_i18n::t!("trigger.rule_name").to_string(),
                    edit_state.name.clone(),
                    p,
                ))
                .child(Self::form_field(
                    rust_i18n::t!("trigger.rule_pattern").to_string(),
                    edit_state.pattern.clone(),
                    p,
                ))
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(
                            div()
                                .text_xs()
                                .text_color(p.muted_foreground)
                                .child(rust_i18n::t!("trigger.case_sensitive").to_string()),
                        )
                        .child(
                            Switch::new("trigger-case-sensitive-switch")
                                .checked(edit_state.case_sensitive)
                                .on_click(cx.listener(|this, &checked, _, cx| {
                                    if let Some(e) = this.modals.settings.trigger_edit.as_mut() {
                                        e.case_sensitive = checked;
                                    }
                                    cx.notify();
                                })),
                        ),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::MEDIUM)
                                .child(rust_i18n::t!("trigger.action_kind").to_string()),
                        )
                        .child(
                            h_flex()
                                .gap_1()
                                .child(
                                    Button::new("action-bell-btn")
                                        .xsmall()
                                        .outline()
                                        .selected(action_kind == ActionKindChoice::Bell)
                                        .label(rust_i18n::t!("trigger.action_bell").to_string())
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.set_trigger_action_kind(
                                                ActionKindChoice::Bell,
                                                cx,
                                            );
                                        })),
                                )
                                .child(
                                    Button::new("action-notify-btn")
                                        .xsmall()
                                        .outline()
                                        .selected(action_kind == ActionKindChoice::Notification)
                                        .label(
                                            rust_i18n::t!("trigger.action_notification")
                                                .to_string(),
                                        )
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.set_trigger_action_kind(
                                                ActionKindChoice::Notification,
                                                cx,
                                            );
                                        })),
                                )
                                .child(
                                    Button::new("action-response-btn")
                                        .xsmall()
                                        .outline()
                                        .selected(action_kind == ActionKindChoice::AutoResponse)
                                        .label(
                                            rust_i18n::t!("trigger.action_auto_response")
                                                .to_string(),
                                        )
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.set_trigger_action_kind(
                                                ActionKindChoice::AutoResponse,
                                                cx,
                                            );
                                        })),
                                ),
                        ),
                );

            if action_kind != ActionKindChoice::Bell {
                form = form.child(Self::form_field(
                    rust_i18n::t!(if action_kind == ActionKindChoice::AutoResponse {
                        "trigger.response_text"
                    } else {
                        "trigger.notification_text"
                    })
                    .to_string(),
                    edit_state.action_text.clone(),
                    p,
                ));
            }

            if let Some(error) = edit_state.error {
                form = form.child(div().text_xs().text_color(p.danger).child(error));
            }

            form = form.child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("cancel-trigger-edit")
                            .small()
                            .label(rust_i18n::t!("common.cancel").to_string())
                            .on_click(cx.listener(|this, _, _, cx| this.cancel_trigger_edit(cx))),
                    )
                    .child(
                        Button::new("save-trigger-edit")
                            .small()
                            .primary()
                            .label(rust_i18n::t!("common.confirm").to_string())
                            .on_click(cx.listener(|this, _, _, cx| this.save_trigger_edit(cx))),
                    ),
            );

            body = body.child(form);
        }

        body.into_any_element()
    }
}
