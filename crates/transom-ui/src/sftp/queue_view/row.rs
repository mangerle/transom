//! SFTP 单个传输任务行的状态与控制操作按钮渲染。
//!
//! 设计原理（Why）：
//! - 实现初衷：将传输队列中单个任务项的复杂状态机匹配、方向图标渲染与操作按钮（重试、暂停、恢复、置顶、取消）独立拆分；
//! - 核心优势：降低主队列容器的圈复杂度，避免大 match 结构膨胀。

use crate::app::AppShell;
use crate::palette::Palette;
use crate::sftp::{TransferKind, TransferStatus, TransferTask};
use gpui_kit::assets::IconName;
use gpui_kit::base::h_flex;
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::{TabId, TransferDirection};

use super::format::{format_eta, format_speed};
use crate::sftp::format_size;
use crate::sftp::transfer_presentation::{display_name, progress_percent, status_label};

/// 渲染单条传输任务行。
pub(super) fn render_queue_task_row(
    idx: usize,
    task: &TransferTask,
    tab_id: TabId,
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let dir_tag = match task.direction {
        TransferDirection::Upload => "↑ 上传",
        TransferDirection::Download => "↓ 下载",
        TransferDirection::Sync => "↕ 同步",
        TransferDirection::RemoteCopy => "⇋ 互传",
    };
    let status_str = match &task.status {
        TransferStatus::Transferring => progress_percent(task).map_or_else(
            || status_label(&task.status),
            |pct| format!("{} ({pct:.0}%)", status_label(&task.status)),
        ),
        _ => status_label(&task.status),
    };
    let file_name = display_name(task);

    let task_id = task.id;
    let status_clone = task.status.clone();
    let mut actions = h_flex().gap_1().items_center();
    match status_clone {
        TransferStatus::Transferring => {
            actions = actions
                .child(
                    Button::new(format!("queue-pause-{idx}"))
                        .ghost()
                        .xsmall()
                        .icon(IconName::Pause)
                        .tooltip("暂停传输")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.sftp_pause_task(tab_id, task_id, cx);
                        })),
                )
                .child(
                    Button::new(format!("queue-cancel-{idx}"))
                        .ghost()
                        .xsmall()
                        .icon(IconName::X)
                        .tooltip("取消任务")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.sftp_cancel_task(tab_id, task_id, cx);
                        })),
                );
        }
        TransferStatus::Paused => {
            actions = actions
                .child(
                    Button::new(format!("queue-prioritize-{idx}"))
                        .ghost()
                        .xsmall()
                        .icon(IconName::ArrowUpToLine)
                        .tooltip("置顶优先传输")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.sftp_prioritize_task(tab_id, task_id, cx);
                        })),
                )
                .child(
                    Button::new(format!("queue-resume-{idx}"))
                        .ghost()
                        .xsmall()
                        .icon(IconName::Play)
                        .tooltip("恢复传输")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.sftp_resume_task(tab_id, task_id, cx);
                        })),
                )
                .child(
                    Button::new(format!("queue-cancel-{idx}"))
                        .ghost()
                        .xsmall()
                        .icon(IconName::X)
                        .tooltip("取消任务")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.sftp_cancel_task(tab_id, task_id, cx);
                        })),
                );
        }
        TransferStatus::Pending => {
            actions = actions
                .child(
                    Button::new(format!("queue-prioritize-{idx}"))
                        .ghost()
                        .xsmall()
                        .icon(IconName::ArrowUpToLine)
                        .tooltip("置顶优先传输")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.sftp_prioritize_task(tab_id, task_id, cx);
                        })),
                )
                .child(
                    Button::new(format!("queue-cancel-{idx}"))
                        .ghost()
                        .xsmall()
                        .icon(IconName::X)
                        .tooltip("取消任务")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.sftp_cancel_task(tab_id, task_id, cx);
                        })),
                );
        }
        TransferStatus::Failed(_) => {
            actions = actions
                .child(
                    Button::new(format!("queue-retry-{idx}"))
                        .ghost()
                        .xsmall()
                        .icon(IconName::RotateCw)
                        .tooltip("重试传输")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.sftp_resume_task(tab_id, task_id, cx);
                        })),
                )
                .child(
                    Button::new(format!("queue-cancel-{idx}"))
                        .ghost()
                        .xsmall()
                        .icon(IconName::X)
                        .tooltip("取消任务")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.sftp_cancel_task(tab_id, task_id, cx);
                        })),
                );
        }
        TransferStatus::Completed | TransferStatus::Cancelled => {
            actions = actions.child(
                Button::new(format!("queue-remove-{idx}"))
                    .ghost()
                    .xsmall()
                    .icon(IconName::Trash)
                    .tooltip("移除记录")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.sftp_remove_task(tab_id, task_id, cx);
                    })),
            );
        }
    }

    h_flex()
        .id(ElementId::Name(format!("queue-task-{idx}").into()))
        .px_2()
        .py_1()
        .gap_2()
        .items_center()
        .text_xs()
        .border_b_1()
        .border_color(p.sidebar_border)
        .hover(|style| style.bg(p.muted))
        .child(
            div()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(if task.direction == TransferDirection::Upload {
                    p.sidebar_accent_foreground
                } else if task.direction == TransferDirection::Download {
                    p.success
                } else if task.direction == TransferDirection::RemoteCopy {
                    p.warning
                } else {
                    p.sidebar_accent_foreground
                })
                .child(dir_tag),
        )
        .when(task.kind == TransferKind::Directory, |this| {
            this.child(
                div()
                    .px_1()
                    .rounded(p.radius.sm)
                    .bg(p.muted)
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("sftp.folder_tag").to_string()),
            )
        })
        .when(task.verify, |this| {
            this.child(
                div()
                    .px_1()
                    .rounded(p.radius.sm)
                    .bg(p.muted)
                    .text_color(p.muted_foreground)
                    .child("SHA256"),
            )
        })
        .child(div().flex_1().overflow_hidden().child(file_name))
        .child(div().text_color(p.muted_foreground).child(match task.size {
            Some(total) => format!("{} / {}", format_size(task.transferred), format_size(total)),
            None => format_size(task.transferred),
        }))
        .when_some(task.speed_bps, |this, speed| {
            this.child(
                div()
                    .text_color(p.sidebar_accent_foreground)
                    .child(format_speed(speed)),
            )
        })
        .when_some(task.eta_secs, |this, eta| {
            this.child(div().text_color(p.muted_foreground).child(format_eta(eta)))
        })
        .child(
            div()
                .text_color(match &task.status {
                    TransferStatus::Transferring => p.sidebar_accent_foreground,
                    TransferStatus::Completed => p.success,
                    TransferStatus::Failed(_) => p.danger,
                    _ => p.muted_foreground,
                })
                .child(status_str),
        )
        .child(actions)
        .into_any_element()
}
