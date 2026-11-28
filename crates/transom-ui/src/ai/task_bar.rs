//! AI 托管任务中心 UI 状态同步、实时滚动日志面板与单任务停止控制。

use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::*;
use transom_core::{AiAppEvent, AiCommand, AiManagedTaskDto, AiManagedTaskStatus};
use transom_ui_icons::{AppIcon, AppIconElement, UiIcon};

use super::state::AiPanelState;
use super::view::AiAssistantView;
use crate::palette::Palette;
use crate::theme::tokens::{space, typography};

const MAX_TASK_LOG_BYTES: usize = 64 * 1024;

/// UI 侧托管任务条目（含实时流式日志缓冲）。
#[derive(Clone, Debug)]
pub(super) struct UiManagedTask {
    pub(super) dto: AiManagedTaskDto,
    pub(super) log: String,
}

/// 将控制平面推送的托管任务事件合并进 `AiPanelState`。
pub(super) fn apply_managed_task_event(state: &mut AiPanelState, event: AiAppEvent) {
    match event {
        AiAppEvent::ManagedTaskStarted(dto) => {
            let id = dto.task_id.clone();
            state.run.managed_tasks.push(UiManagedTask {
                dto,
                log: String::new(),
            });
            if state.run.managed_tasks.len() > 24 {
                let drop_n = state.run.managed_tasks.len() - 24;
                state.run.managed_tasks.drain(..drop_n);
            }
            state.run.selected_task_id = Some(id);
        }
        AiAppEvent::ManagedTaskOutput { task_id, chunk } => {
            if let Some(task) = state
                .run
                .managed_tasks
                .iter_mut()
                .find(|t| t.dto.task_id == task_id)
            {
                task.log.push_str(&chunk);
                if task.log.len() > MAX_TASK_LOG_BYTES {
                    let mut cut = task.log.len() - MAX_TASK_LOG_BYTES;
                    while cut < task.log.len() && !task.log.is_char_boundary(cut) {
                        cut += 1;
                    }
                    task.log.drain(..cut);
                }
            }
        }
        AiAppEvent::ManagedTaskFinished {
            task_id,
            status,
            exit_code,
            summary,
        } => {
            if let Some(task) = state
                .run
                .managed_tasks
                .iter_mut()
                .find(|t| t.dto.task_id == task_id)
            {
                task.dto.status = status;
                task.dto.exit_code = exit_code;
                if !summary.trim().is_empty() && !task.log.ends_with(&summary) {
                    if !task.log.is_empty() && !task.log.ends_with('\n') {
                        task.log.push('\n');
                    }
                    task.log.push_str(&summary);
                }
            }
        }
        _ => {}
    }
}

impl AiAssistantView {
    pub(super) fn render_task_bar_section(
        &self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if self.state.run.managed_tasks.is_empty() {
            return None;
        }
        let running_count = self
            .state
            .run
            .managed_tasks
            .iter()
            .filter(|t| t.dto.status == AiManagedTaskStatus::Running)
            .count();
        if running_count == 0 || !self.state.run.tasks_drawer_expanded {
            return None;
        }
        let summary = rust_i18n::t!("ai.tasks_running_summary", count = running_count).to_string();

        let active_title = self
            .state
            .run
            .managed_tasks
            .iter()
            .rev()
            .find(|t| t.dto.status == AiManagedTaskStatus::Running)
            .map(|t| format!("{} · {}", t.dto.target_label, t.dto.title))
            .unwrap_or_default();

        let v_toggle = cx.entity();
        let expanded = self.state.run.tasks_drawer_expanded;

        let mut card = v_flex()
            .w_full()
            .rounded(p.radius.md)
            .bg(p.surface.floating)
            .border_1()
            .border_color(p.divider(p.border))
            .overflow_hidden();

        let mut header = h_flex()
            .id("ai-task-bar-header")
            .w_full()
            .px(rems(space::SM))
            .py(rems(space::XS))
            .gap(rems(space::XS))
            .items_center()
            .justify_between()
            .cursor_pointer()
            .hover(|s| s.bg(p.surface.hover))
            .on_click(move |_, _, cx| {
                v_toggle.update(cx, |this, cx| {
                    this.state.run.tasks_drawer_expanded = !this.state.run.tasks_drawer_expanded;
                    cx.notify();
                });
            })
            .child(
                h_flex()
                    .flex_1()
                    .gap(px(6.0))
                    .items_center()
                    .overflow_hidden()
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_color(if running_count > 0 {
                                p.primary
                            } else {
                                p.success
                            })
                            .child(
                                AppIconElement::new(AppIcon::Ui(if running_count > 0 {
                                    UiIcon::REFRESH
                                } else {
                                    UiIcon::GREEN_CHECKMARK
                                }))
                                .small(),
                            ),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_size(rems(typography::CAPTION))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(p.foreground)
                            .child(summary),
                    )
                    .child(
                        div()
                            .flex_1()
                            .truncate()
                            .text_size(rems(typography::CAPTION))
                            .text_color(p.text.muted)
                            .child(active_title),
                    ),
            );

        let actions = h_flex()
            .gap(rems(space::XXS))
            .items_center()
            .flex_shrink_0()
            .child(
                div().text_color(p.text.muted).child(
                    AppIconElement::new(AppIcon::Ui(if expanded {
                        UiIcon::CHEVRON_DOWN
                    } else {
                        UiIcon::CHEVRON_RIGHT
                    }))
                    .small(),
                ),
            );
        header = header.child(actions);
        card = card.child(header);

        if expanded {
            card = card.child(self.render_task_drawer_body(p, cx));
        }
        Some(
            div()
                .px(rems(space::SM))
                .pb(rems(space::XS))
                .child(card)
                .into_any_element(),
        )
    }

    fn render_task_drawer_body(&self, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let selected_id = self.state.run.selected_task_id.as_deref().filter(|id| {
            self.state
                .run
                .managed_tasks
                .iter()
                .any(|t| t.dto.task_id == *id)
        });

        let mut list = v_flex()
            .w_full()
            .max_h(rems(7.5))
            .overflow_y_scrollbar()
            .border_t_1()
            .border_color(p.divider(p.border))
            .p(rems(space::XXS))
            .gap(px(2.0));

        for (idx, item) in self
            .state
            .run
            .managed_tasks
            .iter()
            .rev()
            .filter(|t| t.dto.status == AiManagedTaskStatus::Running)
            .enumerate()
        {
            let tid = item.dto.task_id.clone();
            let tid_stop = tid.clone();
            let selected = Some(tid.as_str()) == selected_id;
            let v_pick = cx.entity();
            let v_stop = cx.entity();

            let (icon, color) = match item.dto.status {
                AiManagedTaskStatus::Running => (UiIcon::REFRESH, p.primary),
                AiManagedTaskStatus::Succeeded => (UiIcon::GREEN_CHECKMARK, p.success),
                AiManagedTaskStatus::Failed => (UiIcon::ERROR, p.danger),
                AiManagedTaskStatus::Cancelled => (UiIcon::STOP, p.warning),
            };
            let mut row = h_flex()
                .id(("ai-managed-task-row", idx))
                .w_full()
                .px(rems(space::XS))
                .py(px(3.0))
                .gap(px(6.0))
                .items_center()
                .justify_between()
                .rounded(p.radius.sm)
                .cursor_pointer()
                .hover(|s| s.bg(p.surface.hover))
                .on_click(move |_, _, cx| {
                    let picked = tid.clone();
                    v_pick.update(cx, |this, cx| {
                        if this.state.run.selected_task_id.as_deref() == Some(picked.as_str()) {
                            this.state.run.selected_task_id = None;
                        } else {
                            this.state.run.selected_task_id = Some(picked);
                        }
                        cx.notify();
                    });
                });
            if selected {
                row = row.bg(p.surface.selected);
            }

            row = row.child(
                h_flex()
                    .flex_1()
                    .gap(px(6.0))
                    .items_center()
                    .overflow_hidden()
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_color(color)
                            .child(AppIconElement::new(AppIcon::Ui(icon)).small()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .truncate()
                            .text_size(rems(typography::CAPTION))
                            .text_color(p.foreground)
                            .child(format!("{} · {}", item.dto.target_label, item.dto.title)),
                    ),
            );

            let mut right = h_flex().gap(px(2.0)).items_center().flex_shrink_0();
            if item.dto.status == AiManagedTaskStatus::Running {
                let v_detach = cx.entity();
                let tid_detach = item.dto.task_id.clone();
                let detach_tip = rust_i18n::t!("ai.task_detach").to_string();
                let stop_tip = rust_i18n::t!("ai.task_stop").to_string();
                right = right
                    .child(
                        Button::new(("ai-detach-task", idx))
                            .ghost()
                            .xsmall()
                            .accessibility_label(detach_tip.clone())
                            .tooltip(detach_tip)
                            .child(AppIconElement::new(AppIcon::Ui(UiIcon::EXTERNAL_LINK)).small())
                            .on_click(move |_, _, cx| {
                                let id = tid_detach.clone();
                                v_detach.update(cx, |this, cx| {
                                    this.send_ai_command(
                                        AiCommand::DetachManagedTask { task_id: id },
                                        cx,
                                    );
                                    cx.notify();
                                });
                            }),
                    )
                    .child(
                        Button::new(("ai-stop-task", idx))
                            .ghost()
                            .xsmall()
                            .accessibility_label(stop_tip.clone())
                            .tooltip(stop_tip)
                            .child(
                                div()
                                    .text_color(p.danger)
                                    .child(AppIconElement::new(AppIcon::Ui(UiIcon::STOP)).small()),
                            )
                            .on_click(move |_, _, cx| {
                                let id = tid_stop.clone();
                                v_stop.update(cx, |this, cx| {
                                    this.send_ai_command(
                                        AiCommand::CancelManagedTask { task_id: id },
                                        cx,
                                    );
                                    cx.notify();
                                });
                            }),
                    );
            }
            list = list.child(row.child(right));
        }

        let mut body = v_flex().w_full().child(list);
        if let Some(id) = selected_id
            && let Some(task) = self
                .state
                .run
                .managed_tasks
                .iter()
                .find(|t| t.dto.task_id == id)
        {
            let log_text = if task.log.trim().is_empty() {
                rust_i18n::t!("ai.task_waiting_output").to_string()
            } else {
                task.log.clone()
            };
            body = body.child(
                div()
                    .w_full()
                    .max_h(rems(10.0))
                    .overflow_y_scrollbar()
                    .border_t_1()
                    .border_color(p.divider(p.border))
                    .bg(p.background)
                    .p(rems(space::XS))
                    .font_family("monospace")
                    .text_size(rems(typography::CAPTION))
                    .text_color(p.text.subtle)
                    .child(log_text),
            );
        }
        body.into_any_element()
    }
}
