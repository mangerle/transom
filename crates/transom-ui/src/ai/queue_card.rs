//! 运行中待发送消息队列（Queued Messages）折叠卡片与排队/引导/打断调度。
//!
//! 设计原理（Why）：
//! - 在 AI 运行期间发送新消息不再静默丢弃，而是按用户配置的 [`RunningMessageBehavior`]
//!   分别执行排队（Queue）、中途引导（Steer）或立即打断并发送（Interrupt）；
//! - 提供与主流智能体终端一致的折叠队列卡片：支持展开查看待发列表、单条立即发送（`→`）、
//!   回填编辑（`✎`）与删除（`🗑`）；主动点击停止时自动暂停出队，防止意外连发。

use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::*;
use transom_ai::RunningMessageBehavior;
use transom_core::{AiCommand, AppCommand};
use transom_ui_icons::{AppIcon, AppIconElement, UiIcon};

use super::attachments::ComposerAttachment;
use super::view::AiAssistantView;
use crate::palette::Palette;
use crate::theme::tokens::{space, typography};

/// 队列中等待发送的单条消息载荷（含附件）。
#[derive(Clone, Debug)]
pub(super) struct QueuedPrompt {
    pub(super) id: String,
    pub(super) prompt: String,
    pub(super) attachments: Vec<ComposerAttachment>,
}

/// 渲染输入框正上方的 Queued Messages 折叠队列卡片。
pub(super) fn render_queued_messages_card(
    view: &AiAssistantView,
    p: &Palette,
    cx: &mut Context<AiAssistantView>,
) -> Option<AnyElement> {
    if view.state.run.queued_messages.is_empty()
        || (view.state.has_active_run() && !view.state.active_run_is_current())
    {
        return None;
    }
    let count = view.state.run.queued_messages.len();
    let expanded = view.state.run.queue_expanded;
    let chevron = if expanded {
        UiIcon::CHEVRON_DOWN
    } else {
        UiIcon::CHEVRON_RIGHT
    };
    let status_note = if view.state.run.queue_paused_by_cancel {
        rust_i18n::t!("ai.queue_paused_hint").to_string()
    } else if let Some(focus) = view
        .state
        .run
        .active_focus
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        rust_i18n::t!("ai.queue_waiting_on", focus = focus).to_string()
    } else {
        rust_i18n::t!("ai.queue_waiting_default").to_string()
    };

    let mut card = v_flex()
        .w_full()
        .p(rems(space::XS))
        .gap(px(4.0))
        .rounded(p.radius.md)
        .bg(p.surface.floating)
        .border_1()
        .border_color(p.divider(p.border))
        .child(
            h_flex()
                .id("ai-queue-header")
                .w_full()
                .px(px(4.0))
                .justify_between()
                .items_center()
                .gap(px(6.0))
                .cursor_pointer()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.state.run.queue_expanded = !this.state.run.queue_expanded;
                    cx.notify();
                }))
                .child(
                    h_flex()
                        .flex_1()
                        .gap(px(6.0))
                        .items_center()
                        .overflow_hidden()
                        .child(
                            div()
                                .text_size(rems(typography::CAPTION))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(p.foreground)
                                .child(rust_i18n::t!("ai.queue_title").to_string()),
                        )
                        .child(
                            div()
                                .px(px(6.0))
                                .py(px(1.0))
                                .rounded_full()
                                .bg(p.secondary)
                                .text_size(rems(typography::CAPTION))
                                .text_color(p.foreground)
                                .child(count.to_string()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .truncate()
                                .text_size(rems(typography::CAPTION))
                                .text_color(p.text.muted)
                                .child(status_note),
                        ),
                )
                .child(
                    div()
                        .text_color(p.text.muted)
                        .child(AppIconElement::new(AppIcon::Ui(chevron)).small()),
                ),
        );

    if expanded {
        for (idx, item) in view.state.run.queued_messages.iter().enumerate() {
            let id_send = item.id.clone();
            let id_edit = item.id.clone();
            let id_del = item.id.clone();
            let preview = item.prompt.lines().next().unwrap_or("").trim().to_owned();
            let preview = if preview.is_empty() && !item.attachments.is_empty() {
                rust_i18n::t!("ai.queue_attachments_only", count = item.attachments.len())
                    .to_string()
            } else {
                preview
            };
            let send_tip = rust_i18n::t!("ai.queue_send_now").to_string();
            let edit_tip = rust_i18n::t!("ai.queue_edit").to_string();
            let del_tip = rust_i18n::t!("ai.queue_delete").to_string();

            card = card.child(
                h_flex()
                    .id(("ai-queued-row", idx))
                    .w_full()
                    .px(px(6.0))
                    .py(px(3.0))
                    .justify_between()
                    .items_center()
                    .gap(px(6.0))
                    .rounded(p.radius.sm)
                    .hover(|s| s.bg(p.secondary))
                    .child(
                        div()
                            .flex_1()
                            .truncate()
                            .text_size(rems(typography::CAPTION))
                            .text_color(p.foreground)
                            .child(preview),
                    )
                    .child(
                        h_flex()
                            .flex_shrink_0()
                            .gap(px(2.0))
                            .items_center()
                            .child(
                                Button::new(("ai-q-send", idx))
                                    .ghost()
                                    .xsmall()
                                    .accessibility_label(send_tip.clone())
                                    .tooltip(send_tip)
                                    .child(
                                        AppIconElement::new(AppIcon::Ui(UiIcon::CHEVRON_RIGHT))
                                            .small(),
                                    )
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.send_queued_now(&id_send, cx);
                                    })),
                            )
                            .child(
                                Button::new(("ai-q-edit", idx))
                                    .ghost()
                                    .xsmall()
                                    .accessibility_label(edit_tip.clone())
                                    .tooltip(edit_tip)
                                    .child(AppIconElement::new(AppIcon::Ui(UiIcon::EDIT)).small())
                                    .on_click(cx.listener(move |this, _, w, cx| {
                                        this.edit_queued_item(&id_edit, w, cx);
                                    })),
                            )
                            .child(
                                Button::new(("ai-q-del", idx))
                                    .ghost()
                                    .xsmall()
                                    .accessibility_label(del_tip.clone())
                                    .tooltip(del_tip)
                                    .child(AppIconElement::new(AppIcon::Ui(UiIcon::DELETE)).small())
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.remove_queued_item(&id_del, cx);
                                    })),
                            ),
                    ),
            );
        }
    }

    Some(
        div()
            .px(rems(space::SM))
            .pb(rems(space::XS))
            .child(card)
            .into_any_element(),
    )
}

impl AiAssistantView {
    /// 当运行中发送消息时，按设置项选择排队、中途引导或打断立即发送。
    pub(super) fn handle_running_message(&mut self, prompt: String, cx: &mut Context<Self>) {
        let attachments = std::mem::take(&mut self.state.composer.pending_attachments);
        match self.state.config.settings.running_message_behavior {
            RunningMessageBehavior::Queue => {
                self.state.run.queue_paused_by_cancel = false;
                self.state.run.queued_messages.push(QueuedPrompt {
                    id: uuid::Uuid::new_v4().to_string(),
                    prompt,
                    attachments,
                });
                cx.notify();
            }
            RunningMessageBehavior::Steer => {
                if let Some(run_id) = self.state.current_run_id().map(str::to_owned) {
                    let _ =
                        self.command_tx
                            .try_send(AppCommand::Ai(Box::new(AiCommand::SteerRun {
                                run_id,
                                message: prompt,
                            })));
                    self.state.run.active_focus =
                        Some(rust_i18n::t!("ai.steer_injected").to_string());
                    cx.notify();
                }
            }
            RunningMessageBehavior::Interrupt => {
                self.state.run.queue_paused_by_cancel = false;
                self.state.run.pending_interrupt_send = Some(QueuedPrompt {
                    id: uuid::Uuid::new_v4().to_string(),
                    prompt,
                    attachments,
                });
                self.cancel_running(cx);
            }
        }
    }

    pub(super) fn send_queued_now(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(pos) = self
            .state
            .run
            .queued_messages
            .iter()
            .position(|q| q.id == id)
        else {
            return;
        };
        let item = self.state.run.queued_messages.remove(pos);
        self.state.run.queue_paused_by_cancel = false;
        if self.state.active_run_is_current() {
            self.state.run.pending_interrupt_send = Some(item);
            self.cancel_running(cx);
        } else {
            self.dispatch_queued_item(item, cx);
        }
    }

    pub(super) fn edit_queued_item(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(pos) = self
            .state
            .run
            .queued_messages
            .iter()
            .position(|q| q.id == id)
        else {
            return;
        };
        let item = self.state.run.queued_messages.remove(pos);
        self.state.composer.pending_attachments = item.attachments;
        self.composer_input
            .update(cx, |s, cx| s.set_value(item.prompt, window, cx));
        cx.notify();
    }

    pub(super) fn remove_queued_item(&mut self, id: &str, cx: &mut Context<Self>) {
        self.state.run.queued_messages.retain(|q| q.id != id);
        cx.notify();
    }

    pub(super) fn dispatch_queued_item(&mut self, item: QueuedPrompt, cx: &mut Context<Self>) {
        self.state.composer.pending_attachments = item.attachments;
        self.send_prompt_text(item.prompt, cx);
    }

    pub(super) fn maybe_drain_queue_after_finish(
        &mut self,
        cancelled: bool,
        cx: &mut Context<Self>,
    ) {
        if let Some(interrupt_item) = self.state.run.pending_interrupt_send.take() {
            self.dispatch_queued_item(interrupt_item, cx);
            return;
        }
        if cancelled || self.state.run.queue_paused_by_cancel {
            return;
        }
        if !self.state.run.queued_messages.is_empty() {
            let next = self.state.run.queued_messages.remove(0);
            self.dispatch_queued_item(next, cx);
        }
    }

    pub(super) fn set_running_message_behavior(
        &mut self,
        behavior: RunningMessageBehavior,
        cx: &mut Context<Self>,
    ) {
        self.state.config.settings.running_message_behavior = behavior;
        self.persist_settings_reported(cx);
    }

    pub(super) fn respond_approval(
        &mut self,
        request_id: &str,
        decision: transom_core::AiApprovalDecision,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let matched = self
            .state
            .run
            .pending_approvals
            .iter()
            .find(|a| a.request_id == request_id)
            .cloned();
        let handoff_cmd = matched
            .as_ref()
            .map(|a| a.detail.clone())
            .unwrap_or_default();
        if let Some(approval) = matched {
            let (approved, label) = match &decision {
                transom_core::AiApprovalDecision::AllowOnce => (
                    true,
                    rust_i18n::t!("ai.approval_allowed_once", title = approval.title.as_str())
                        .to_string(),
                ),
                transom_core::AiApprovalDecision::AllowPrefix { prefix } => (
                    true,
                    rust_i18n::t!("ai.approval_allowed_prefix", prefix = prefix.as_str())
                        .to_string(),
                ),
                transom_core::AiApprovalDecision::HandoffToTerminal => (
                    true,
                    rust_i18n::t!(
                        "ai.approval_handoff_manual",
                        title = approval.title.as_str()
                    )
                    .to_string(),
                ),
                transom_core::AiApprovalDecision::RejectWithFeedback { reason } => {
                    let r = reason.trim();
                    if r.is_empty() {
                        (
                            false,
                            rust_i18n::t!("ai.approval_rejected", title = approval.title.as_str())
                                .to_string(),
                        )
                    } else {
                        (
                            false,
                            rust_i18n::t!(
                                "ai.approval_rejected_reason",
                                title = approval.title.as_str(),
                                reason = r
                            )
                            .to_string(),
                        )
                    }
                }
            };
            self.state
                .conversation
                .items
                .push(super::state_types::UiChatItem::AuditNotice {
                    id: format!("audit-{}", self.state.conversation.items.len()),
                    approved,
                    label,
                    host_label: approval.host_label,
                });
        }
        self.state
            .run
            .pending_approvals
            .retain(|a| a.request_id != request_id);
        self.approval_feedback_input
            .update(cx, |s, cx| s.set_value("", window, cx));
        if matches!(
            decision,
            transom_core::AiApprovalDecision::HandoffToTerminal
        ) {
            self.handle_handoff_focus_request(handoff_cmd, cx);
        }
        let _ = self
            .command_tx
            .try_send(AppCommand::Ai(Box::new(AiCommand::RespondApproval {
                request_id: request_id.to_owned(),
                decision,
            })));
        cx.notify();
    }

    pub(super) fn cancel_running(&mut self, cx: &mut Context<Self>) {
        if let Some(run_id) = self.state.request_current_run_cancel() {
            if self.state.run.pending_interrupt_send.is_none() {
                self.state.run.queue_paused_by_cancel = true;
            }
            if let Err(err) = self
                .command_tx
                .try_send(AppCommand::Ai(Box::new(AiCommand::CancelRun { run_id })))
            {
                self.state.view.status_error = Some(
                    rust_i18n::t!("ai.cancel_run_failed", error = err.to_string()).to_string(),
                );
            }
        }
        cx.notify();
    }
}
