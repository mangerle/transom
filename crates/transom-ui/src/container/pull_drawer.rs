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
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::ContainerCommand;
use transom_ui_icons::AppIcon;
use transom_ui_icons::AppIconElement;
use transom_ui_icons::UiIcon;

/// 渲染镜像下载管理抽屉。
///
/// 设计原理（Why）：抽屉只依赖实体状态与 `Palette`，动作全部回到
/// [`ContainerWorkspace`]；外壳只读一次状态并传入句柄，不再把渲染方法挂在
/// `AppShell` 上。回调经实体句柄 `update`，不经过外壳。
pub(crate) fn render_pull_drawer(
    view: &Entity<ContainerWorkspace>,
    workspace: &ContainerWorkspaceState,
    p: &Palette,
) -> AnyElement {
    if !workspace.pull_drawer_open {
        return div().into_any_element();
    }
    let tasks = workspace.pull_manager.tasks.clone();
    let pulling_count = workspace.pull_manager.pulling_count();
    let queued_count = tasks
        .iter()
        .filter(|task| task.state == PullTaskState::Queued)
        .count();
    let has_finished = tasks.iter().any(|task| task.state.is_finished());

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
                    AppIconElement::new(AppIcon::Ui(UiIcon::DOWNLOAD))
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
                            .child(AppIconElement::new(AppIcon::Ui(UiIcon::DELETE)).small())
                            .accessibility_label(
                                rust_i18n::t!("container.pull_manager.clear_finished").to_string(),
                            )
                            .tooltip(
                                rust_i18n::t!("container.pull_manager.clear_finished").to_string(),
                            )
                            .on_click({
                                let view = view.clone();
                                move |_, _, cx| {
                                    view.update(cx, |workspace, cx| {
                                        workspace.state.pull_manager.clear_finished();
                                        cx.notify();
                                    });
                                }
                            }),
                    )
                })
                .child(
                    Button::new("pull-drawer-close")
                        .ghost()
                        .xsmall()
                        .child(AppIconElement::new(AppIcon::Ui(UiIcon::CLOSE)).small())
                        .accessibility_label(
                            rust_i18n::t!("container.pull_manager.close_drawer").to_string(),
                        )
                        .tooltip(rust_i18n::t!("container.pull_manager.close_drawer").to_string())
                        .on_click({
                            let view = view.clone();
                            move |_, _, cx| {
                                view.update(cx, |workspace, cx| {
                                    workspace.state.pull_drawer_open = false;
                                    cx.notify();
                                });
                            }
                        }),
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
                AppIconElement::new(AppIcon::Ui(UiIcon::CHECK))
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
                                percent = format!("{:>3.0}", r * 100.0)
                            )
                            .to_string()
                        })
                        .unwrap_or_else(|| state.label()),
                ),
                PullTaskState::Success => (p.success, state.label()),
                PullTaskState::Failed(_) => (p.danger, state.label()),
                PullTaskState::Cancelled => (p.muted_foreground, state.label()),
            };

            let is_finished = state.is_finished();
            let mut card = v_flex()
                .w_full()
                .p_2p5()
                .gap_1p5()
                .rounded(px(10.0))
                .bg(p.surface.ghost);

            // 完成态平滑淡出：正常显示为弱化半透明，鼠标悬停时完全聚焦
            if is_finished {
                card = card.opacity(0.55).hover(|s| s.opacity(1.0));
            }

            card = card.child(
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
                                    .font_family(p.mono_font_family.clone())
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
                                .on_click({
                                    let view = view.clone();
                                    let cancel_task_id = cancel_task_id.clone();
                                    move |_, _, cx| {
                                        view.update(cx, |workspace, cx| {
                                            let next_cmd = workspace
                                                .state
                                                .pull_manager
                                                .cancel_task(&cancel_task_id);
                                            // 发送取消中断指令
                                            workspace.send(ContainerCommand::CancelPullTask {
                                                tab_id: workspace.tab_id,
                                                task_id: cancel_task_id.clone(),
                                            });
                                            // 调度下一个任务
                                            if let Some((next_id, next_tab, next_ref, next_auth)) =
                                                next_cmd
                                            {
                                                workspace.send(ContainerCommand::PullImageTask {
                                                    tab_id: next_tab,
                                                    task_id: next_id,
                                                    reference: next_ref,
                                                    auth: next_auth,
                                                });
                                            }
                                            cx.notify();
                                        });
                                    }
                                })
                            } else {
                                let remove_task_id = task_id.clone();
                                Button::new(ElementId::Name(
                                    format!("pull-remove-{}", remove_task_id).into(),
                                ))
                                .ghost()
                                .xsmall()
                                .child(AppIconElement::new(AppIcon::Ui(UiIcon::DELETE)).small())
                                .accessibility_label("Remove pull task")
                                .on_click({
                                    let view = view.clone();
                                    let remove_task_id = remove_task_id.clone();
                                    move |_, _, cx| {
                                        view.update(cx, |workspace, cx| {
                                            workspace
                                                .state
                                                .pull_manager
                                                .remove_task(&remove_task_id);
                                            cx.notify();
                                        });
                                    }
                                })
                            }),
                    ),
            );

            // 胶囊进度条与分层状态渲染
            if state == PullTaskState::Pulling {
                let progress_val = ratio.unwrap_or(0.0).clamp(0.0, 1.0);
                card = card.child(
                    v_flex()
                        .w_full()
                        .gap_1()
                        .child(capsule_progress_bar(progress_val, p.primary))
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

/// 胶囊进度条：6px 高度、全圆角、8% 弱色轨道底。
fn capsule_progress_bar(fraction: f32, color: Hsla) -> AnyElement {
    let progress_val = fraction.clamp(0.0, 1.0);
    div()
        .w_full()
        .h(px(6.0))
        .rounded_full()
        .bg(color.opacity(0.08))
        .child(
            div()
                .h_full()
                .w(relative(progress_val))
                .rounded_full()
                .bg(color),
        )
        .into_any_element()
}
