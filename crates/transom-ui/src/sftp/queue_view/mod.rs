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

use crate::app::app_settings;
use crate::app::update_app_settings;
use crate::divider::{DividerEdge, DividerStyled};
use crate::palette::Palette;
use crate::sftp::entity::SftpPanelView;
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

impl SftpPanelView {
    pub(crate) fn render_sftp_queue_pane(
        &self,
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
            for task in queue {
                queue_rows.push(render_queue_task_row(task, tab_id, p, cx));
            }
        }

        let failed_count = queue
            .iter()
            .filter(|t| matches!(t.status, TransferStatus::Failed(_)))
            .count();

        let mut queue_pane = v_flex();
        // 队列收起时没有拖拽条，顶部分割线由容器边框提供；展开时视觉线由
        // 拖拽条自己绘制，避免边框与命中带叠成「线 + 空白条」或拖拽时双倍粗。
        if !queue_expanded {
            queue_pane = queue_pane
                .border_color(p.divider(p.sidebar_border))
                .divider_edge(DividerEdge::Top, p.divider_width);
        }
        queue_pane
            .when(queue_expanded, |this| {
                this.child(
                    h_flex()
                        .id(ElementId::Name("sftp-queue-splitter".into()))
                        .w_full()
                        // 视觉高度与分割线档位一致，命中带不再额外占一行空白。
                        .h(px(p.divider_width.pixels()))
                        .cursor_row_resize()
                        .bg(p.divider(p.sidebar_border))
                        // hover 只换色提示可拖拽，不加高、不叠边框。
                        .hover(|s| s.bg(p.muted_foreground))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                                this.start_resize_queue(event.position.y.into(), cx);
                            }),
                        ),
                )
            })
            .child(self.render_queue_header(
                QueueHeaderParams {
                    tab_id,
                    queue_len: queue.len(),
                    resume_enabled,
                    verify_after_transfer,
                    max_concurrent_transfers,
                    queue_expanded,
                    failed_count,
                },
                p,
                cx,
            ))
            .when(queue_expanded, |this| {
                this.child(
                    v_flex()
                        .id(ElementId::Name("sftp-queue-list".into()))
                        .h(px(queue_height))
                        // 标题栏与第一个任务行之间补一条线：行与行之间本来就有
                        // 底线，列表顶部不画会让标题栏和首行直接贴在一起。
                        .border_color(p.divider(p.sidebar_border))
                        .divider_edge(DividerEdge::Top, p.divider_width)
                        .overflow_y_scrollbar()
                        .children(queue_rows),
                )
            })
            .into_any_element()
    }

    /// 队列标题栏：标题、恢复/校验开关、并发、失败重试与折叠动作。
    fn render_queue_header(
        &self,
        params: QueueHeaderParams,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
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
                    .child(self.render_queue_switches(params, p, cx))
                    .child(self.render_concurrency_button(params, cx)),
            )
            .child(self.render_queue_controls(params, cx))
            .into_any_element()
    }

    /// 队列标题、恢复开关与传输后校验开关。
    fn render_queue_switches(
        &self,
        params: QueueHeaderParams,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let QueueHeaderParams {
            queue_len,
            resume_enabled,
            verify_after_transfer,
            ..
        } = params;

        h_flex()
            .gap_2()
            .items_center()
            .child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(format!(
                        "{} ({})",
                        rust_i18n::t!("sftp.queue.title"),
                        queue_len
                    )),
            )
            .child(
                h_flex()
                    .gap_1p5()
                    .items_center()
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child(rust_i18n::t!("sftp.resume_transfer").to_string()),
                    )
                    .child(
                        Switch::new("toggle-resume")
                            .small()
                            .checked(resume_enabled)
                            .on_click(cx.listener(move |this, &checked, _, cx| {
                                this.state.queue.resume_enabled = checked;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                h_flex()
                    .gap_1p5()
                    .items_center()
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child(rust_i18n::t!("sftp.verify_after_transfer").to_string()),
                    )
                    .child(
                        Switch::new("toggle-verify-after-transfer")
                            .small()
                            .checked(verify_after_transfer)
                            .on_click(cx.listener(move |_this, &checked, _, cx| {
                                update_app_settings(cx, |s| {
                                    s.sftp.verify_after_transfer = checked;
                                });
                                let algo = app_settings(cx).sftp.checksum_algorithm.label();
                                let status = if checked {
                                    rust_i18n::t!("sftp.queue.verify_on", algo = algo).to_string()
                                } else {
                                    rust_i18n::t!("sftp.queue.verify_off", algo = algo).to_string()
                                };
                                cx.emit(crate::sftp::SftpPanelEvent::PersistSettings { status });
                                cx.notify();
                            })),
                    ),
            )
            .into_any_element()
    }

    /// 并发数循环切换按钮。
    fn render_concurrency_button(
        &self,
        params: QueueHeaderParams,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let QueueHeaderParams {
            tab_id,
            max_concurrent_transfers,
            ..
        } = params;

        Button::new("sftp-concurrency")
            .ghost()
            .xsmall()
            .label(
                rust_i18n::t!("sftp.queue.concurrency", count = max_concurrent_transfers)
                    .to_string(),
            )
            .tooltip(rust_i18n::t!("sftp.queue.concurrency_tooltip").to_string())
            .on_click(cx.listener(move |this, _, _, cx| {
                this.cycle_concurrency(tab_id, cx);
            }))
            .into_any_element()
    }

    /// 失败重试、过滤规则、清理已完成与折叠动作。
    fn render_queue_controls(
        &self,
        params: QueueHeaderParams,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let QueueHeaderParams {
            tab_id,
            queue_expanded,
            failed_count,
            ..
        } = params;

        h_flex()
            .gap_1()
            .when(failed_count > 0, |this| {
                this.child(
                    Button::new("retry-all-failed")
                        .ghost()
                        .xsmall()
                        .icon(IconName::RotateCw)
                        .label(
                            rust_i18n::t!("sftp.queue.retry_failed", count = failed_count)
                                .to_string(),
                        )
                        .tooltip(rust_i18n::t!("sftp.queue.retry_all_tooltip").to_string())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.retry_all_failed(tab_id, cx);
                        })),
                )
            })
            .child(
                crate::widgets::IconButton::new(
                    "sftp-filter-rules",
                    IconName::ListFilter,
                    rust_i18n::t!("sftp.queue.filter_rules_tooltip").to_string(),
                )
                .size(crate::widgets::IconButtonSize::Compact)
                .render()
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.open_filter_modal(window, cx);
                })),
            )
            .child(
                Button::new("clear-queue-done")
                    .ghost()
                    .xsmall()
                    .label(rust_i18n::t!("sftp.clear_done").to_string())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.queue_clear_done(cx);
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
                    .accessibility_label(if queue_expanded {
                        rust_i18n::t!("sftp.queue.collapse").to_string()
                    } else {
                        rust_i18n::t!("sftp.queue.expand").to_string()
                    })
                    .tooltip(if queue_expanded {
                        rust_i18n::t!("sftp.queue.collapse").to_string()
                    } else {
                        rust_i18n::t!("sftp.queue.expand").to_string()
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.state.queue.expanded = !this.state.queue.expanded;
                        cx.notify();
                    })),
            )
            .into_any_element()
    }
}

/// 队列标题栏参数：计数与开关值都由调用方读好。
#[derive(Clone, Copy)]
struct QueueHeaderParams {
    tab_id: transom_core::TabId,
    queue_len: usize,
    resume_enabled: bool,
    verify_after_transfer: bool,
    max_concurrent_transfers: usize,
    queue_expanded: bool,
    failed_count: usize,
}

#[cfg(test)]
mod tests;
