//! SFTP 传输队列与底部状态栏视图渲染。

use super::format_size;
use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::progress::Progress;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::switch::Switch;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::{
    TabId, TransferDirection, TransferId, TransferKind, TransferStatus, TransferTask,
};

pub(crate) struct SftpStatusBarProps {
    pub(crate) tab_id: TabId,
    pub(crate) loading: bool,
    pub(crate) progress: Option<(u64, Option<u64>)>,
    pub(crate) active_op: Option<TransferId>,
    pub(crate) remote_error: Option<String>,
    pub(crate) local_error: Option<String>,
    pub(crate) status: Option<String>,
}

pub(crate) struct SftpQueuePaneProps<'a> {
    pub(crate) tab_id: TabId,
    pub(crate) queue: &'a [TransferTask],
    pub(crate) resume_enabled: bool,
    pub(crate) queue_expanded: bool,
    pub(crate) queue_height: f32,
}

impl AppShell {
    pub(crate) fn render_sftp_queue_pane(
        &mut self,
        props: SftpQueuePaneProps<'_>,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tab_id = props.tab_id;
        let queue = props.queue;
        let resume_enabled = props.resume_enabled;
        let queue_expanded = props.queue_expanded;
        let queue_height = props.queue_height;
        let mut queue_rows: Vec<AnyElement> = Vec::with_capacity(queue.len().max(1));
        if queue.is_empty() {
            queue_rows.push(
                div()
                    .px_3()
                    .py_2()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("sftp.no_transfers").to_string())
                    .into_any_element(),
            );
        } else {
            for (idx, task) in queue.iter().enumerate() {
                let dir_tag = match task.direction {
                    TransferDirection::Upload => "↑ 上传",
                    TransferDirection::Download => "↓ 下载",
                };
                let status_str = match &task.status {
                    TransferStatus::Pending => rust_i18n::t!("sftp.pending").to_string(),
                    TransferStatus::Transferring => {
                        if let Some(pct) = task.size.and_then(|total| {
                            task.transferred.saturating_mul(100).checked_div(total)
                        }) {
                            format!("{} ({pct}%)", rust_i18n::t!("sftp.transferring"))
                        } else {
                            rust_i18n::t!("sftp.transferring").to_string()
                        }
                    }
                    TransferStatus::Paused => "暂停".to_string(),
                    TransferStatus::Completed => "已完成".to_string(),
                    TransferStatus::Failed(err) => format!("失败: {err}"),
                    TransferStatus::Cancelled => "已取消".to_string(),
                };
                let file_name = task
                    .local_path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| task.remote_path.clone());

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

                queue_rows.push(
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
                                } else {
                                    p.success
                                })
                                .child(dir_tag),
                        )
                        .when(task.kind == TransferKind::Directory, |this| {
                            this.child(
                                div()
                                    .px_1()
                                    .rounded_sm()
                                    .bg(p.muted)
                                    .text_color(p.muted_foreground)
                                    .child(rust_i18n::t!("sftp.folder_tag").to_string()),
                            )
                        })
                        .child(div().flex_1().overflow_hidden().child(file_name))
                        .child(div().text_color(p.muted_foreground).child(match task.size {
                            Some(total) => format!(
                                "{} / {}",
                                format_size(task.transferred),
                                format_size(total)
                            ),
                            None => format_size(task.transferred),
                        }))
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
                        .into_any_element(),
                );
            }
        }

        let failed_count = queue
            .iter()
            .filter(|t| matches!(t.status, TransferStatus::Failed(_)))
            .count();

        v_flex()
            .border_t_1()
            .border_color(p.sidebar_border)
            .when(queue_expanded, |this| {
                this.child(
                    h_flex()
                        .id(ElementId::Name("sftp-queue-splitter".into()))
                        .w_full()
                        .h(px(3.0))
                        .cursor_row_resize()
                        .hover(|s| s.bg(p.sidebar_border))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                                this.sftp_start_resize_queue(tab_id, event.position.y.into(), cx);
                            }),
                        ),
                )
            })
            .child(
                h_flex()
                    .px_3()
                    .py_1()
                    .items_center()
                    .justify_between()
                    .bg(p.muted)
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(div().text_xs().font_weight(FontWeight::SEMIBOLD).child(
                                format!("{} ({})", rust_i18n::t!("sftp.queue"), queue.len()),
                            ))
                            .child(
                                h_flex()
                                    .gap_1p5()
                                    .items_center()
                                    .child(
                                        div().text_xs().text_color(p.muted_foreground).child(
                                            rust_i18n::t!("sftp.resume_transfer").to_string(),
                                        ),
                                    )
                                    .child(
                                        Switch::new("toggle-resume")
                                            .small()
                                            .checked(resume_enabled)
                                            .on_click(cx.listener(move |this, &checked, _, cx| {
                                                if let Some(panel) = this
                                                    .tabs
                                                    .iter_mut()
                                                    .find(|tab| tab.id == tab_id)
                                                    .and_then(|tab| tab.sftp.as_mut())
                                                {
                                                    panel.resume_enabled = checked;
                                                    cx.notify();
                                                }
                                            })),
                                    ),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .when(failed_count > 0, |this| {
                                this.child(
                                    Button::new("retry-all-failed")
                                        .ghost()
                                        .xsmall()
                                        .icon(IconName::RotateCw)
                                        .label(format!("重试失败 ({failed_count})"))
                                        .tooltip("一键重试队列中所有失败的传输任务")
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.sftp_retry_all_failed(tab_id, cx);
                                        })),
                                )
                            })
                            .child(
                                Button::new("sftp-filter-rules")
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::ListFilter)
                                    .tooltip("传输过滤规则（仅对目录传输生效）")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.sftp_open_filter_modal(window, cx);
                                    })),
                            )
                            .child(
                                Button::new("clear-queue-done")
                                    .ghost()
                                    .xsmall()
                                    .label(rust_i18n::t!("sftp.clear_done").to_string())
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.sftp_queue_clear_done(tab_id, cx);
                                    })),
                            )
                            .child(
                                Button::new("toggle-queue-exp")
                                    .ghost()
                                    .xsmall()
                                    .icon(if queue_expanded {
                                        IconName::ChevronDown
                                    } else {
                                        IconName::ChevronUp
                                    })
                                    .tooltip(if queue_expanded {
                                        "收起传输列表"
                                    } else {
                                        "展开传输列表"
                                    })
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if let Some(panel) = this
                                            .tabs
                                            .iter_mut()
                                            .find(|tab| tab.id == tab_id)
                                            .and_then(|tab| tab.sftp.as_mut())
                                        {
                                            panel.queue_expanded = !panel.queue_expanded;
                                            cx.notify();
                                        }
                                    })),
                            ),
                    ),
            )
            .when(queue_expanded, |this| {
                this.child(
                    v_flex()
                        .id(ElementId::Name("sftp-queue-list".into()))
                        .h(px(queue_height))
                        .overflow_y_scrollbar()
                        .children(queue_rows),
                )
            })
            .into_any_element()
    }

    pub(crate) fn render_sftp_status_bar(
        &mut self,
        props: SftpStatusBarProps,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let tab_id = props.tab_id;
        if props.loading {
            Some(
                v_flex()
                    .px_3()
                    .py_1p5()
                    .gap_1()
                    .bg(p.muted)
                    .child(
                        div()
                            .text_xs()
                            .child(rust_i18n::t!("sftp.loading").to_string()),
                    )
                    .child(Progress::new("sftp-loading-progress").loading(true))
                    .into_any_element(),
            )
        } else if let Some((transferred, total)) = props.progress {
            let percent = if let Some(tot) = total {
                if tot > 0 {
                    ((transferred as f64 / tot as f64) * 100.0) as f32
                } else {
                    0.0
                }
            } else {
                0.0
            };
            let progress_str = match total {
                Some(tot) => format!(
                    "{} / {} ({:.1}%)",
                    format_size(transferred),
                    format_size(tot),
                    percent
                ),
                None => format_size(transferred),
            };
            let mut header = h_flex()
                .justify_between()
                .items_center()
                .child(div().text_xs().child(progress_str));
            if let Some(op_id) = props.active_op {
                header = header.child(
                    h_flex()
                        .gap_1()
                        .items_center()
                        .child(
                            Button::new("top-sftp-pause")
                                .ghost()
                                .xsmall()
                                .icon(IconName::Pause)
                                .tooltip("暂停传输")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.sftp_pause_task(tab_id, op_id, cx);
                                })),
                        )
                        .child(
                            Button::new("top-sftp-cancel")
                                .ghost()
                                .xsmall()
                                .icon(IconName::X)
                                .tooltip("取消传输")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.sftp_cancel_task(tab_id, op_id, cx);
                                })),
                        ),
                );
            }
            Some(
                v_flex()
                    .px_3()
                    .py_1p5()
                    .gap_1()
                    .bg(p.muted)
                    .child(header)
                    .child(Progress::new("sftp-transfer-progress").value(percent))
                    .into_any_element(),
            )
        } else if let Some(error) = props.remote_error.or(props.local_error) {
            Some(
                div()
                    .px_3()
                    .py_1()
                    .text_xs()
                    .text_color(p.danger)
                    .bg(p.muted)
                    .child(error)
                    .into_any_element(),
            )
        } else {
            props.status.map(|s| {
                div()
                    .px_3()
                    .py_1()
                    .text_xs()
                    .bg(p.muted)
                    .child(s)
                    .into_any_element()
            })
        }
    }
}
