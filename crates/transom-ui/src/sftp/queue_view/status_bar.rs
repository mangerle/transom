//! SFTP 底部状态栏与传输进度条渲染。
//!
//! 设计原理（Why）：
//! - 实现初衷：为 SFTP 双栏文件浏览器底部提供常驻的状态显示、错误提示及传输进度条；
//! - 核心优势：
//!   1. 集中处理 loading、progress、error 与普通 status 优先级的展示分发；
//!   2. 支持一键全部暂停、全部恢复与全部取消。

use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::progress::Progress;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::TabId;

use super::format::format_speed;
use crate::sftp::format_size;

pub(crate) struct SftpStatusBarProps {
    pub(crate) tab_id: TabId,
    pub(crate) loading: bool,
    pub(crate) progress: Option<(u64, Option<u64>)>,
    /// 当前活动传输任务数。
    pub(crate) active_count: usize,
    /// 当前暂停任务数。
    pub(crate) paused_count: usize,
    /// 传输队列是否展开；展开时底部只保留进度条，避免与队列行重复。
    pub(crate) queue_expanded: bool,
    pub(crate) remote_error: Option<String>,
    pub(crate) local_error: Option<String>,
    pub(crate) status: Option<String>,
    /// 实时总传输速率。
    pub(crate) total_speed_bps: Option<u64>,
}

impl AppShell {
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
            let progress_bar = Progress::new("sftp-transfer-progress").value(percent);

            if props.queue_expanded {
                // 队列展开时底部保留进度条与总速率小字
                return Some(
                    v_flex()
                        .px_3()
                        .py_1p5()
                        .gap_1()
                        .bg(p.muted)
                        .child(progress_bar)
                        .when_some(props.total_speed_bps, |this, speed| {
                            this.child(
                                h_flex()
                                    .justify_between()
                                    .text_xs()
                                    .text_color(p.muted_foreground)
                                    .child(format!("总传输速率: {}", format_speed(speed)))
                                    .child(format!("{:.1}%", percent)),
                            )
                        })
                        .into_any_element(),
                );
            }

            let progress_str = match total {
                Some(tot) => format!(
                    "{} / {} ({:.1}%)",
                    format_size(transferred),
                    format_size(tot),
                    percent
                ),
                None => format_size(transferred),
            };
            let speed_suffix = props
                .total_speed_bps
                .map(|s| format!(" · {}", format_speed(s)))
                .unwrap_or_default();
            let progress_str = if props.active_count > 0 {
                format!(
                    "{} 个任务传输中{speed_suffix} · {progress_str}",
                    props.active_count
                )
            } else if props.paused_count > 0 {
                format!("已暂停 {} 个任务 · {progress_str}", props.paused_count)
            } else {
                progress_str
            };
            let mut header = h_flex()
                .justify_between()
                .items_center()
                .child(div().text_xs().child(progress_str));
            if props.active_count > 0 {
                header = header.child(
                    h_flex()
                        .gap_1()
                        .items_center()
                        .child(
                            Button::new("top-sftp-pause-all")
                                .ghost()
                                .xsmall()
                                .icon(IconName::Pause)
                                .tooltip("暂停全部传输")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.sftp_pause_all(tab_id, cx);
                                })),
                        )
                        .child(
                            Button::new("top-sftp-cancel-all")
                                .ghost()
                                .xsmall()
                                .icon(IconName::X)
                                .tooltip("取消全部传输")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.sftp_cancel_all(tab_id, cx);
                                })),
                        ),
                );
            } else if props.paused_count > 0 {
                header = header.child(
                    h_flex()
                        .gap_1()
                        .items_center()
                        .child(
                            Button::new("top-sftp-resume-all")
                                .ghost()
                                .xsmall()
                                .icon(IconName::Play)
                                .tooltip("继续全部传输")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.sftp_resume_all(tab_id, cx);
                                })),
                        )
                        .child(
                            Button::new("top-sftp-cancel-paused-all")
                                .ghost()
                                .xsmall()
                                .icon(IconName::X)
                                .tooltip("取消全部传输")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.sftp_cancel_all(tab_id, cx);
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
                    .child(progress_bar)
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
