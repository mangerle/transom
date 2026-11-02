//! 触发器设置页渲染：规则列表、启用开关与内联编辑器。

use super::*;
use super::{trigger_editor_fields, trigger_rule_row};

impl SettingsView {
    pub(crate) fn render_settings_triggers(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let enabled = app_settings(cx).triggers.enabled;
        let rules = app_settings(cx).triggers.rules.clone();
        let edit = self.state.trigger_edit.clone();

        let list = self.render_trigger_list(&rules, p, cx);

        let mut body = v_flex()
            .gap_3()
            .child(crate::widgets::setting_row(
                "trigger.master_switch",
                Some("trigger.master_switch_desc"),
                Switch::new("toggle-triggers-master")
                    .checked(enabled)
                    .on_click(cx.listener(|this, &checked, _, cx| {
                        update_app_settings(cx, |s| s.triggers.enabled = checked);
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

        if let Some(edit_state) = edit {
            body = body.child(self.render_trigger_editor(edit_state, p, cx));
        }

        body.into_any_element()
    }

    /// 触发器规则列表：逐条开关、编辑与删除。
    fn render_trigger_list(
        &self,
        rules: &[transom_core::TriggerRule],
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
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
            list = list.child(trigger_rule_row(rule, p, cx));
        }

        list.into_any_element()
    }

    /// 内联编辑表单：模式、动作与校验错误。
    /// 动作类型三选一：响铃、通知、自动响应。
    fn render_action_kind_selector(
        &self,
        action_kind: ActionKindChoice,
        cx: &mut Context<Self>,
    ) -> AnyElement {
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
                                this.set_trigger_action_kind(ActionKindChoice::Bell, cx);
                            })),
                    )
                    .child(
                        Button::new("action-notify-btn")
                            .xsmall()
                            .outline()
                            .selected(action_kind == ActionKindChoice::Notification)
                            .label(rust_i18n::t!("trigger.action_notification").to_string())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.set_trigger_action_kind(ActionKindChoice::Notification, cx);
                            })),
                    )
                    .child(
                        Button::new("action-response-btn")
                            .xsmall()
                            .outline()
                            .selected(action_kind == ActionKindChoice::AutoResponse)
                            .label(rust_i18n::t!("trigger.action_auto_response").to_string())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.set_trigger_action_kind(ActionKindChoice::AutoResponse, cx);
                            })),
                    ),
            )
            .into_any_element()
    }

    fn render_trigger_editor(
        &self,
        edit_state: crate::trigger_panel::TriggerEditState,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let action_kind = edit_state.action_kind;

        let mut form = v_flex()
            .p_3()
            .rounded_lg()
            .border_1()
            .border_color(p.border)
            .bg(p.popover)
            .gap_2p5()
            .child(trigger_editor_fields(&edit_state, p))
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
                                if let Some(e) = this.state.trigger_edit.as_mut() {
                                    e.case_sensitive = checked;
                                }
                                cx.notify();
                            })),
                    ),
            )
            .child(self.render_action_kind_selector(action_kind, cx));

        if action_kind != ActionKindChoice::Bell {
            form = form.child(crate::widgets::form_field(
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

        form.into_any_element()
    }
}
