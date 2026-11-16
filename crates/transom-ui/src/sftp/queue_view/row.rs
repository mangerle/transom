//! SFTP 单个传输任务行的状态与控制操作按钮渲染。
//!
//! 设计原理（Why）：
//! - 实现初衷：将传输队列中单个任务项的复杂状态机匹配、方向图标渲染与操作按钮（重试、暂停、恢复、置顶、取消）独立拆分；
//! - 核心优势：降低主队列容器的圈复杂度，避免大 match 结构膨胀。

use crate::divider::{DividerEdge, DividerStyled};
use crate::palette::Palette;
use crate::sftp::entity::SftpPanelView;
use crate::sftp::{TransferKind, TransferStatus, TransferTask};
use gpui_kit::base::h_flex;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::{TabId, TransferDirection};
use transom_ui_icons::AppIcon;
use transom_ui_icons::UiIcon;

use super::format::{format_eta, format_speed};
use crate::sftp::format_size;
use crate::sftp::transfer_presentation::{display_name, progress_percent, status_label};

/// 渲染单条传输任务行。
pub(super) fn render_queue_task_row(
    task: &TransferTask,
    tab_id: TabId,
    p: &Palette,
    cx: &mut Context<SftpPanelView>,
) -> AnyElement {
    let dir_tag = match task.direction {
        TransferDirection::Upload => rust_i18n::t!("sftp.queue.dir_upload").to_string(),
        TransferDirection::Download => rust_i18n::t!("sftp.queue.dir_download").to_string(),
        TransferDirection::Sync => rust_i18n::t!("sftp.queue.dir_sync").to_string(),
        TransferDirection::RemoteCopy => rust_i18n::t!("sftp.queue.dir_remote_copy").to_string(),
    };
    let status_str = match &task.status {
        TransferStatus::Transferring => progress_percent(task).map_or_else(
            || status_label(&task.status),
            |pct| format!("{} ({pct:.0}%)", status_label(&task.status)),
        ),
        _ => status_label(&task.status),
    };
    let task_id = task.id;
    let actions = render_queue_task_actions(task_id, &task.status, tab_id, cx);

    h_flex()
        .id(ElementId::Name(format!("queue-task-{task_id}").into()))
        .px_2()
        .py_1()
        .gap_2()
        .items_center()
        .text_xs()
        .border_color(p.divider(p.sidebar_border))
        .divider_edge(DividerEdge::Bottom, p.divider_width)
        .hover(|style| style.bg(p.muted))
        .child(
            div()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(match task.direction {
                    TransferDirection::Download => p.success,
                    TransferDirection::RemoteCopy => p.warning,
                    TransferDirection::Upload | TransferDirection::Sync => {
                        p.sidebar_accent_foreground
                    }
                })
                .child(dir_tag),
        )
        .when(task.kind == TransferKind::Directory, |this| {
            this.child(queue_tag_badge(
                rust_i18n::t!("sftp.folder_tag").to_string(),
                p,
            ))
        })
        .when(task.verify.enabled, |this| {
            this.child(queue_tag_badge(task.verify.algorithm.label(), p))
        })
        .child(div().flex_1().overflow_hidden().child(display_name(task)))
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
                    TransferStatus::Transferring | TransferStatus::Verifying => {
                        p.sidebar_accent_foreground
                    }
                    TransferStatus::Completed => p.success,
                    TransferStatus::Failed(_) => p.danger,
                    _ => p.muted_foreground,
                })
                .child(status_str),
        )
        .child(actions)
        .into_any_element()
}

fn queue_tag_badge(label: impl Into<SharedString>, p: &Palette) -> AnyElement {
    div()
        .px_1()
        .rounded(p.radius.sm)
        .bg(p.muted)
        .text_color(p.muted_foreground)
        .child(label.into())
        .into_any_element()
}

fn queue_action_btn(
    id: String,
    icon: UiIcon,
    label: String,
    show_tooltip: bool,
    on_click: impl Fn(&mut SftpPanelView, &mut Context<SftpPanelView>) + 'static,
    cx: &mut Context<SftpPanelView>,
) -> AnyElement {
    crate::widgets::IconButton::new(id, AppIcon::Ui(icon), label)
        .size(crate::widgets::IconButtonSize::Compact)
        .show_tooltip(show_tooltip)
        .render()
        .on_click(cx.listener(move |this, _, _, cx| on_click(this, cx)))
        .into_any_element()
}

fn render_queue_task_actions(
    task_id: transom_core::TransferId,
    status: &TransferStatus,
    tab_id: TabId,
    cx: &mut Context<SftpPanelView>,
) -> AnyElement {
    let pause = |cx: &mut Context<SftpPanelView>| {
        queue_action_btn(
            format!("queue-pause-{task_id}"),
            UiIcon::PAUSE,
            rust_i18n::t!("sftp.queue.pause_task").to_string(),
            true,
            move |this, cx| this.pause_task(tab_id, task_id, cx),
            cx,
        )
    };
    let cancel = |show_tip: bool, cx: &mut Context<SftpPanelView>| {
        queue_action_btn(
            format!("queue-cancel-{task_id}"),
            UiIcon::CLOSE,
            rust_i18n::t!("sftp.queue.cancel_task").to_string(),
            show_tip,
            move |this, cx| this.cancel_task(tab_id, task_id, cx),
            cx,
        )
    };
    let prioritize = |cx: &mut Context<SftpPanelView>| {
        queue_action_btn(
            format!("queue-prioritize-{task_id}"),
            UiIcon::UPLOAD,
            rust_i18n::t!("sftp.queue.prioritize_task").to_string(),
            true,
            move |this, cx| this.prioritize_task(tab_id, task_id, cx),
            cx,
        )
    };
    let resume =
        |label_key: &str, id_prefix: &str, icon: UiIcon, cx: &mut Context<SftpPanelView>| {
            queue_action_btn(
                format!("{id_prefix}-{task_id}"),
                icon,
                rust_i18n::t!(label_key).to_string(),
                true,
                move |this, cx| this.resume_task(tab_id, task_id, cx),
                cx,
            )
        };
    let remove = |cx: &mut Context<SftpPanelView>| {
        queue_action_btn(
            format!("queue-remove-{task_id}"),
            UiIcon::DELETE,
            rust_i18n::t!("sftp.overlay_remove").to_string(),
            true,
            move |this, cx| this.remove_task(task_id, cx),
            cx,
        )
    };

    let row = h_flex().gap_1().items_center();
    match status {
        TransferStatus::Transferring => row.child(pause(cx)).child(cancel(false, cx)),
        TransferStatus::Verifying => row.child(cancel(true, cx)),
        TransferStatus::Paused => row
            .child(prioritize(cx))
            .child(resume(
                "sftp.queue.resume_task",
                "queue-resume",
                UiIcon::RUN,
                cx,
            ))
            .child(cancel(true, cx)),
        TransferStatus::Pending => row.child(prioritize(cx)).child(cancel(true, cx)),
        TransferStatus::Failed(_) => row
            .child(resume(
                "sftp.queue.retry_task",
                "queue-retry",
                UiIcon::REFRESH,
                cx,
            ))
            // 失败任务直接可移除：取消只是把状态改成“已取消”，还得再清空
            // 队列才能清掉失败记录，对只想收起失败项的用户是多余的步骤。
            .child(remove(cx))
            .child(cancel(true, cx)),
        TransferStatus::Completed | TransferStatus::Cancelled => row.child(remove(cx)),
    }
    .into_any_element()
}
