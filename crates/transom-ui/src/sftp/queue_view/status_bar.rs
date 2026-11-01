//! SFTP 底部状态栏与传输进度条渲染。
//!
//! 设计原理（Why）：
//! - 实现初衷：为 SFTP 双栏文件浏览器底部提供常驻的状态显示、错误提示及传输进度条；
//! - 核心优势：
//!   1. 集中处理 progress、error 与普通 status 优先级的展示分发；
//!   2. 支持一键全部暂停、全部恢复与全部取消；
//!   3. 避免目录读取等瞬态侵入状态栏导致整个 SFTP 面板布局抖动与闪烁。

use crate::palette::Palette;
use crate::sftp::entity::SftpPanelView;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::progress::Progress;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::TabId;

use super::format::format_speed;
use crate::sftp::format_size;

pub(crate) struct SftpStatusBarProps {
    pub(crate) tab_id: TabId,
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

impl SftpPanelView {
    pub(crate) fn render_sftp_status_bar(
        &self,
        props: SftpStatusBarProps,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let tab_id = props.tab_id;
        if let Some((transferred, total)) = props.progress {
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
                return Some(expanded_progress_bar(percent, props.total_speed_bps, p));
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
                rust_i18n::t!(
                    "sftp.queue.active_progress",
                    count = props.active_count,
                    suffix = speed_suffix.as_str(),
                    progress = progress_str.as_str()
                )
                .to_string()
            } else if props.paused_count > 0 {
                rust_i18n::t!(
                    "sftp.queue.paused_progress",
                    count = props.paused_count,
                    progress = progress_str.as_str()
                )
                .to_string()
            } else {
                progress_str
            };
            let mut header = h_flex()
                .justify_between()
                .items_center()
                .child(div().text_xs().child(progress_str));
            if let Some(actions) = self.progress_actions(&props, tab_id, cx) {
                header = header.child(actions);
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

    /// 传输中的批量动作：活动任务给暂停/取消，暂停任务给恢复/取消。
    fn progress_actions(
        &self,
        props: &SftpStatusBarProps,
        tab_id: TabId,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if props.active_count > 0 {
            Some(
                h_flex()
                    .gap_1()
                    .items_center()
                    .child(
                        crate::widgets::IconButton::new(
                            "top-sftp-pause-all",
                            IconName::Pause,
                            rust_i18n::t!("sftp.queue.pause_all").to_string(),
                        )
                        .size(crate::widgets::IconButtonSize::Compact)
                        .render()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.pause_all(tab_id, cx);
                        })),
                    )
                    .child(
                        crate::widgets::IconButton::new(
                            "top-sftp-cancel-all",
                            IconName::X,
                            rust_i18n::t!("sftp.queue.cancel_all").to_string(),
                        )
                        .size(crate::widgets::IconButtonSize::Compact)
                        .render()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.cancel_all(tab_id, cx);
                        })),
                    )
                    .into_any_element(),
            )
        } else if props.paused_count > 0 {
            Some(
                h_flex()
                    .gap_1()
                    .items_center()
                    .child(
                        crate::widgets::IconButton::new(
                            "top-sftp-resume-all",
                            IconName::Play,
                            rust_i18n::t!("sftp.queue.resume_all").to_string(),
                        )
                        .size(crate::widgets::IconButtonSize::Compact)
                        .render()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.resume_all(tab_id, cx);
                        })),
                    )
                    .child(
                        crate::widgets::IconButton::new(
                            "top-sftp-cancel-paused-all",
                            IconName::X,
                            rust_i18n::t!("sftp.queue.cancel_all").to_string(),
                        )
                        .size(crate::widgets::IconButtonSize::Compact)
                        .render()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.cancel_all(tab_id, cx);
                        })),
                    )
                    .into_any_element(),
            )
        } else {
            None
        }
    }
}

/// 队列展开时的底部进度条：进度与总速率小字。
fn expanded_progress_bar(percent: f32, total_speed_bps: Option<u64>, p: &Palette) -> AnyElement {
    v_flex()
        .px_3()
        .py_1p5()
        .gap_1()
        .bg(p.muted)
        .child(Progress::new("sftp-transfer-progress").value(percent))
        .when_some(total_speed_bps, |this, speed| {
            let speed_text = format_speed(speed);
            this.child(
                h_flex()
                    .justify_between()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(
                        rust_i18n::t!("sftp.queue.total_speed", speed = speed_text.as_str())
                            .to_string(),
                    )
                    .child(format!("{:.1}%", percent)),
            )
        })
        .into_any_element()
}
