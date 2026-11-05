//! 镜像下载管理器抽屉与分层进度跟踪面板。
//!
//! 设计原理（Why）：
//! - 将所有并发下载与排队任务集中在常驻悬浮抽屉中呈现，用户在切换分类或执行其他操作时
//!   仍能对后台拉取进度一目了然；
//! - 各 Layer（分层）独立渲染其状态（Downloading / Extracting / Complete），并实时计算
//!   总体下载进度百分比；
//! - 提供单任务中断取消与一键清理已完成任务，避免内存与状态列表无限膨胀。

use super::*;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::{ContainerCommand, TabId};

impl AppShell {
    /// 渲染镜像下载管理抽屉。
    pub(crate) fn render_container_pull_drawer(
        &mut self,
        tab_id: TabId,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let Some(workspace) = self.state.container_workspace(tab_id) else {
            return div().into_any_element();
        };

        if !workspace.pull_drawer_open {
            return div().into_any_element();
        }

        let tasks = workspace.pull_manager.tasks.clone();
        let pulling_count = workspace.pull_manager.pulling_count();
        let queued_count = tasks
            .iter()
            .filter(|t| t.state == PullTaskState::Queued)
            .count();
        let has_finished = tasks.iter().any(|t| t.state.is_finished());

        // 头部统计与操作
        let header = h_flex()
            .w_full()
            .px_3()
            .py_2()
            .items_center()
            .justify_between()
            .border_b_1()
            .border_color(p.border)
            .bg(p.background)
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Icon::new(IconName::Download)
                            .small()
                            .text_color(p.primary),
                    )
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::BOLD)
                            .text_color(p.foreground)
                            .child(rust_i18n::t!("container.pull_manager.center_title").to_string()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child(rust_i18n::t!(
                                "container.pull_manager.summary_counts",
                                pulling = pulling_count,
                                queued = queued_count
                            )),
                    ),
            )
            .child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .when(has_finished, |this| {
                        this.child(
                            Button::new("pull-drawer-clear")
                                .ghost()
                                .xsmall()
                                .icon(IconName::Trash)
                                .tooltip(rust_i18n::t!("container.pull_manager.clear_finished").to_string())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(ws) = this.state.container_workspace_mut(tab_id) {
                                        ws.pull_manager.clear_finished();
                                    }
                                    cx.notify();
                                })),
                        )
                    })
                    .child(
                        Button::new("pull-drawer-close")
                            .ghost()
                            .xsmall()
                            .icon(IconName::X)
                            .tooltip(rust_i18n::t!("container.pull_manager.close_drawer").to_string())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(ws) = this.state.container_workspace_mut(tab_id) {
                                    ws.pull_drawer_open = false;
                                }
                                cx.notify();
                            })),
                    ),
            );

        // 任务列表
        let list: AnyElement = if tasks.is_empty() {
            v_flex()
                .size_full()
                .h(px(160.0))
                .items_center()
                .justify_center()
                .gap_2()
                .child(
                    Icon::new(IconName::Check)
                        .with_size(px(24.0))
                        .text_color(p.muted_foreground),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(rust_i18n::t!("container.pull_manager.empty_tasks").to_string()),
                )
                .into_any_element()
        } else {
            let mut items = v_flex().gap_2().w_full().p_2();
            for task in tasks.iter().rev() {
                let task_id = task.id.clone();
                let reference = task.reference.clone();
                let reg_name = task.registry_name.clone();
                let state = task.state.clone();
                let status_line = task.status_line.clone();
                let ratio = task.progress_ratio();
                let is_active = state.is_active();

                let (state_color, state_label) = match &state {
                    PullTaskState::Queued => (p.warning, state.label()),
                    PullTaskState::Pulling => (
                        p.primary,
                        ratio
                            .map(|r| {
                                rust_i18n::t!(
                                    "container.pull_manager.downloading_percent",
                                    percent = format!("{:.0}", r * 100.0)
                                )
                                .to_string()
                            })
                            .unwrap_or_else(|| state.label()),
                    ),
                    PullTaskState::Success => (p.success, state.label()),
                    PullTaskState::Failed(_) => (p.danger, state.label()),
                    PullTaskState::Cancelled => (p.muted_foreground, state.label()),
                };

                let mut card = v_flex()
                    .w_full()
                    .p_2p5()
                    .gap_1p5()
                    .rounded(p.radius.md)
                    .border_1()
                    .border_color(p.border)
                    .bg(p.background)
                    .child(
                        h_flex()
                            .w_full()
                            .items_center()
                            .justify_between()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .items_center()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(p.foreground)
                                            .truncate()
                                            .child(reference),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .px_1p5()
                                            .py_0p5()
                                            .rounded(p.radius.sm)
                                            .bg(p.muted.opacity(0.5))
                                            .text_color(p.muted_foreground)
                                            .child(reg_name),
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
                                            .text_color(state_color)
                                            .child(state_label),
                                    )
                                    .child(if is_active {
                                        let cancel_task_id = task_id.clone();
                                        Button::new(ElementId::Name(
                                            format!("pull-cancel-{}", cancel_task_id).into(),
                                        ))
                                        .danger()
                                        .xsmall()
                                        .label(rust_i18n::t!("container.pull_manager.cancel").to_string())
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            let next_cmd = if let Some(ws) =
                                                this.state.container_workspace_mut(tab_id)
                                            {
                                                ws.pull_manager.cancel_task(&cancel_task_id)
                                            } else {
                                                None
                                            };
                                            // 发送取消中断指令
                                            this.state.send_command(
                                                ContainerCommand::CancelPullTask {
                                                    tab_id,
                                                    task_id: cancel_task_id.clone(),
                                                },
                                            );
                                            // 调度下一个任务
                                            if let Some((next_id, next_tab, next_ref)) = next_cmd {
                                                this.state.send_command(
                                                    ContainerCommand::PullImageTask {
                                                        tab_id: next_tab,
                                                        task_id: next_id,
                                                        reference: next_ref,
                                                        auth: None,
                                                    },
                                                );
                                            }
                                            cx.notify();
                                        }))
                                    } else {
                                        let remove_task_id = task_id.clone();
                                        Button::new(ElementId::Name(
                                            format!("pull-remove-{}", remove_task_id).into(),
                                        ))
                                        .ghost()
                                        .xsmall()
                                        .icon(IconName::Trash)
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            if let Some(ws) =
                                                this.state.container_workspace_mut(tab_id)
                                            {
                                                ws.pull_manager.remove_task(&remove_task_id);
                                            }
                                            cx.notify();
                                        }))
                                    }),
                            ),
                    );

                // 进度条渲染
                if state == PullTaskState::Pulling {
                    let progress_val = ratio.unwrap_or(0.0).clamp(0.0, 1.0);
                    card = card.child(
                        v_flex()
                            .w_full()
                            .gap_1()
                            .child(crate::widgets::progress_bar(progress_val, p.primary, p))
                            .when(!status_line.is_empty(), |this| {
                                this.child(
                                    div()
                                        .text_xs()
                                        .text_color(p.muted_foreground)
                                        .truncate()
                                        .child(status_line),
                                )
                            }),
                    );
                }

                items = items.child(card);
            }

            div()
                .id("pull-drawer-scroll")
                .w_full()
                .max_h(px(320.0))
                .overflow_y_scroll()
                .child(items)
                .into_any_element()
        };

        // 浮动抽屉容器
        div()
            .id("container-pull-drawer-container")
            .absolute()
            .right(px(16.0))
            .bottom(px(16.0))
            .w(px(420.0))
            .rounded(p.radius.lg)
            .shadow_lg()
            .border_1()
            .border_color(p.border)
            .bg(p.background)
            .overflow_hidden()
            .child(header)
            .child(list)
            .into_any_element()
    }
}
