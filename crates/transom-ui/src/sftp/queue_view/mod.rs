//! SFTP 传输队列折叠面板与控制工具栏渲染。
//!
//! 设计原理（Why）：
//! - 实现初衷：为 SFTP 传输任务提供底部可伸缩抽屉面板，支持批量管理、断点续传与并发调节；
//! - 核心优势：
//!   1. 任务列表与控制栏高内聚；
//!   2. 支持一键重试失败任务、清理已完成项与一键直达过滤规则弹窗。

mod format;
mod row;
mod status_bar;

use self::row::render_queue_task_row;
pub(crate) use self::status_bar::SftpStatusBarProps;

use crate::app::AppShell;
use crate::palette::{Palette, localized_error};
use crate::sftp::{TransferStatus, TransferTask};
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::switch::Switch;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::TabId;

pub(crate) struct SftpQueuePaneProps<'a> {
    pub(crate) tab_id: TabId,
    pub(crate) queue: &'a [TransferTask],
    pub(crate) resume_enabled: bool,
    pub(crate) verify_after_transfer: bool,
    pub(crate) max_concurrent_transfers: usize,
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
        let verify_after_transfer = props.verify_after_transfer;
        let max_concurrent_transfers = props.max_concurrent_transfers;
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
                queue_rows.push(render_queue_task_row(idx, task, tab_id, p, cx));
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
                                                    .workspace
                                                    .tab_mut(tab_id)
                                                    .and_then(|tab| tab.sftp_mut())
                                                {
                                                    panel.queue.resume_enabled = checked;
                                                    cx.notify();
                                                }
                                            })),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .gap_1p5()
                                    .items_center()
                                    .child(div().text_xs().text_color(p.muted_foreground).child(
                                        rust_i18n::t!("sftp.verify_after_transfer").to_string(),
                                    ))
                                    .child(
                                        Switch::new("toggle-verify-after-transfer")
                                            .small()
                                            .checked(verify_after_transfer)
                                            .on_click(cx.listener(move |this, &checked, _, cx| {
                                                this.settings.sftp.verify_after_transfer = checked;
                                                match this
                                                    .services
                                                    .settings_store
                                                    .save(&this.settings)
                                                {
                                                    Ok(()) => {
                                                        this.status = Some(
                                                            if checked {
                                                                "已开启传输后 SHA256 校验"
                                                            } else {
                                                                "已关闭传输后 SHA256 校验"
                                                            }
                                                            .to_string(),
                                                        );
                                                    }
                                                    Err(error) => {
                                                        this.set_error(localized_error(&error));
                                                    }
                                                }
                                                cx.notify();
                                            })),
                                    ),
                            )
                            .child(
                                Button::new("sftp-concurrency")
                                    .ghost()
                                    .xsmall()
                                    .label(format!("并发 {max_concurrent_transfers}"))
                                    .tooltip("同时传输的任务数，点击切换 1-4")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.sftp_cycle_concurrency(cx);
                                    })),
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
                                            .workspace
                                            .tab_mut(tab_id)
                                            .and_then(|tab| tab.sftp_mut())
                                        {
                                            panel.queue.expanded = !panel.queue.expanded;
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
}
