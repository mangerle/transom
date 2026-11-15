//! SFTP 拖拽上传浮层的纯视图渲染。
//!
//! 设计原理（Why）：渲染仅读取 `Entity<SftpPanelView>` 快照并把按钮回调直接
//! 派发回实体自身，由实体发出 `SftpPanelEvent::OverlayChanged` 触发外壳重绘，
//! 彻底消除渲染函数对主壳层的反向依赖。

use super::super::entity::SftpPanelView;
use super::super::format_size;
use super::super::transfer_presentation::{display_name, is_busy, progress_percent, status_label};
use super::super::{SftpPanelEvent, TransferStatus, TransferTask};
use crate::palette::Palette;
use crate::status_bar::format_rate;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::progress::Progress;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::TabId;
use transom_ui_icons::{AppIcon, AppIconElement, UiIcon};

/// 明细区最大高度：几十个文件时不能把整个终端遮住。
const OVERLAY_MAX_HEIGHT: Pixels = px(240.0);
/// 浮层与窗格右下内边缘的距离，与终端搜索条的留白同量级。
const OVERLAY_INSET: Pixels = px(18.0);
/// 单行进度条宽度。
const OVERLAY_BAR_WIDTH: Pixels = px(96.0);

/// 浮层标题参数：标签、展开态与汇总文案。
struct OverlayHeader {
    tab_id: TabId,
    expanded: bool,
    summary: String,
    counters: String,
}

/// 渲染窗格终端区右下角的上传浮层；不该显示时返回 `None`。
pub(crate) fn render_drop_overlay(
    view: &Entity<SftpPanelView>,
    tab_id: TabId,
    p: &Palette,
    cx: &App,
) -> Option<AnyElement> {
    let panel = &view.read(cx).state;
    if panel.open || !panel.queue.drop_overlay.visible || panel.queue.tasks.is_empty() {
        return None;
    }
    let expanded = panel.queue.drop_overlay.expanded;
    let tasks = panel.queue.tasks.clone();
    let total = tasks.len();
    let busy = tasks.iter().filter(|task| is_busy(task)).count();
    let speed = panel.queue.drop_overlay.rate_bytes_per_sec();
    let (done_bytes, total_bytes) = panel.queue.progress.unwrap_or((0, None));
    // 目标目录挂在 tooltip 上：浮层面积有限，但"传到哪儿"必须能查到。
    let target_tip = format!(
        "{} · {}",
        rust_i18n::t!("sftp.overlay_target"),
        panel.remote.path
    );
    let summary = rust_i18n::t!("sftp.overlay_summary", busy = busy, total = total).to_string();
    let counters = match total_bytes {
        Some(bytes) => format!(
            " · {}/{} · {}",
            format_size(done_bytes),
            format_size(bytes),
            format_rate(speed)
        ),
        // 目录树任务拿不到总大小，只报已传字节与速率。
        None => format!(" · {} · {}", format_size(done_bytes), format_rate(speed)),
    };

    let header = render_overlay_header(
        view,
        OverlayHeader {
            tab_id,
            expanded,
            summary,
            counters,
        },
        p,
    );
    let bulk_actions = render_overlay_bulk_actions(view, tab_id, busy);

    let detail = v_flex()
        .gap_1()
        .max_h(OVERLAY_MAX_HEIGHT)
        .overflow_y_scrollbar()
        .when(expanded, |this| {
            this.children(tasks.iter().map(|task| overlay_row(view, tab_id, task, p)))
        });

    // 绝对定位挂在窗格的终端容器上（见 workspace/panes/mod.rs 的 .relative()），
    // 因此 bottom/right 直接对齐终端区右下角，不受侧栏与面板列宽度影响。
    Some(
        v_flex()
            .id(ElementId::Name(format!("drop-overlay-{tab_id}").into()))
            .absolute()
            .bottom(OVERLAY_INSET)
            .right(OVERLAY_INSET)
            .w(rems(22.0))
            .gap_1()
            .p_2()
            .rounded(p.radius.none)
            .bg(p.popover)
            .border_1()
            .border_color(p.border)
            .shadow_lg()
            .tooltip(move |_window, cx| Tooltip::new(target_tip.clone()).build(_window, cx))
            .child(header)
            .child(bulk_actions)
            .child(detail)
            .into_any_element(),
    )
}

/// 浮层标题：上传图标、汇总/计数器、展开与关闭。
fn render_overlay_header(
    view: &Entity<SftpPanelView>,
    header_input: OverlayHeader,
    p: &Palette,
) -> AnyElement {
    let OverlayHeader {
        tab_id,
        expanded,
        summary,
        counters,
    } = header_input;
    let view_toggle = view.clone();
    let view_dismiss = view.clone();
    h_flex()
        .w_full()
        .items_center()
        .gap_1()
        .child(
            AppIconElement::new(AppIcon::Ui(UiIcon::UPLOAD))
                .xsmall()
                .text_color(p.sidebar_accent_foreground),
        )
        .child(
            div()
                .min_w_0()
                .flex_1()
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .truncate()
                .child(format!("{summary}{counters}")),
        )
        .child(
            crate::widgets::IconButton::new(
                format!("overlay-toggle-{tab_id}"),
                if expanded {
                    AppIcon::Ui(UiIcon::CHEVRON_DOWN)
                } else {
                    AppIcon::Ui(UiIcon::CHEVRON_RIGHT)
                },
                rust_i18n::t!("sftp.overlay_expand"),
            )
            .size(crate::widgets::IconButtonSize::Compact)
            .render()
            .on_click(move |_, _, cx| {
                view_toggle.update(cx, |view, cx| view.toggle_drop_overlay_expanded(cx));
            }),
        )
        .child(
            crate::widgets::IconButton::new(
                format!("overlay-dismiss-{tab_id}"),
                AppIcon::Ui(UiIcon::CLOSE),
                rust_i18n::t!("sftp.overlay_dismiss"),
            )
            .size(crate::widgets::IconButtonSize::Compact)
            .render()
            .on_click(move |_, _, cx| {
                view_dismiss.update(cx, |view, cx| view.dismiss_drop_overlay(cx));
            }),
        )
        .into_any_element()
}

/// 浮层批量动作：有活动任务时暂停/取消全部。
fn render_overlay_bulk_actions(
    view: &Entity<SftpPanelView>,
    tab_id: TabId,
    busy: usize,
) -> AnyElement {
    let view_pause = view.clone();
    let view_cancel = view.clone();
    h_flex()
        .gap_0p5()
        .when(busy > 0, |this| {
            this.child(
                crate::widgets::IconButton::new(
                    format!("overlay-pause-all-{tab_id}"),
                    AppIcon::Ui(UiIcon::PAUSE),
                    rust_i18n::t!("sftp.overlay_pause_all"),
                )
                .size(crate::widgets::IconButtonSize::Compact)
                .render()
                .on_click(move |_, _, cx| {
                    view_pause.update(cx, |view, cx| {
                        view.pause_all(tab_id, cx);
                        view.sync_drop_overlay(cx);
                        cx.emit(SftpPanelEvent::OverlayChanged);
                    });
                }),
            )
            .child(
                crate::widgets::IconButton::new(
                    format!("overlay-cancel-all-{tab_id}"),
                    AppIcon::Ui(UiIcon::ABORT),
                    rust_i18n::t!("sftp.overlay_cancel_all"),
                )
                .size(crate::widgets::IconButtonSize::Compact)
                .render()
                .on_click(move |_, _, cx| {
                    view_cancel.update(cx, |view, cx| {
                        view.cancel_all(tab_id, cx);
                        view.sync_drop_overlay(cx);
                        cx.emit(SftpPanelEvent::OverlayChanged);
                    });
                }),
            )
        })
        .into_any_element()
}

/// 一行的明细：文件名 + 进度条 + 状态文案 + 与该状态相称的动作。
///
/// 动作集合与面板队列区同口径（在途→暂停/取消，暂停→继续/取消，结束→移除），
/// 免得同一个任务在两处能做的事不一样。
fn overlay_row(
    view: &Entity<SftpPanelView>,
    tab_id: TabId,
    task: &TransferTask,
    p: &Palette,
) -> AnyElement {
    let task_id = task.id;
    let name = display_name(task);
    // 百分比与 `Progress::value` 同用 0..100 口径；目录树任务没有总大小。
    let percent = progress_percent(task);
    let status_text = match &task.status {
        TransferStatus::Transferring => percent.map_or_else(
            || status_label(&task.status),
            |value| format!("{value:.0}% · {}", format_size(task.transferred)),
        ),
        // 浮层空间窄，失败只显示原因，前缀由状态图标表达。
        TransferStatus::Failed(failure) => failure.reason().to_string(),
        _ => status_label(&task.status),
    };
    let (icon, icon_color) = match &task.status {
        TransferStatus::Failed(_) => (AppIcon::Ui(UiIcon::ERROR), p.danger),
        TransferStatus::Cancelled => (AppIcon::Ui(UiIcon::ABORT), p.muted_foreground),
        TransferStatus::Completed => (AppIcon::Ui(UiIcon::CHECK), p.success),
        TransferStatus::Paused => (AppIcon::Ui(UiIcon::PAUSE), p.warning),
        TransferStatus::Verifying => (AppIcon::Ui(UiIcon::LOCKED), p.sidebar_accent_foreground),
        _ => (AppIcon::Ui(UiIcon::UPLOAD), p.sidebar_accent_foreground),
    };
    let bar = match &task.status {
        TransferStatus::Pending
        | TransferStatus::Transferring
        | TransferStatus::Verifying
        | TransferStatus::Paused => Progress::new(format!("overlay-progress-{task_id}"))
            .w(OVERLAY_BAR_WIDTH)
            .rounded_full()
            .value(percent.unwrap_or(0.0))
            .into_any_element(),
        // 结束态没有进度可显示，用等宽占位保持各行右侧对齐。
        _ => div().w(OVERLAY_BAR_WIDTH).into_any_element(),
    };
    let paused = task.status == TransferStatus::Paused;
    let view_toggle = view.clone();
    let view_cancel = view.clone();
    let view_remove = view.clone();

    let actions = h_flex()
        .gap_0p5()
        .when(is_busy(task), |this| {
            let is_verifying = task.status == TransferStatus::Verifying;
            this.when(!is_verifying, |this| {
                this.child(
                    crate::widgets::IconButton::new(
                        format!("overlay-act-{task_id}-toggle"),
                        if paused {
                            AppIcon::Ui(UiIcon::RUN)
                        } else {
                            AppIcon::Ui(UiIcon::PAUSE)
                        },
                        rust_i18n::t!(if paused {
                            "sftp.overlay_resume"
                        } else {
                            "sftp.overlay_pause"
                        }),
                    )
                    .size(crate::widgets::IconButtonSize::Compact)
                    .render()
                    .on_click(move |_, _, cx| {
                        view_toggle.update(cx, |view, cx| {
                            if paused {
                                view.resume_task(tab_id, task_id, cx);
                            } else {
                                view.pause_task(tab_id, task_id, cx);
                            }
                            view.sync_drop_overlay(cx);
                            cx.emit(SftpPanelEvent::OverlayChanged);
                        });
                    }),
                )
            })
            .child(
                crate::widgets::IconButton::new(
                    format!("overlay-act-{task_id}-cancel"),
                    AppIcon::Ui(UiIcon::CLOSE),
                    rust_i18n::t!("sftp.overlay_cancel"),
                )
                .size(crate::widgets::IconButtonSize::Compact)
                .render()
                .on_click(move |_, _, cx| {
                    view_cancel.update(cx, |view, cx| {
                        view.cancel_task(tab_id, task_id, cx);
                        view.sync_drop_overlay(cx);
                        cx.emit(SftpPanelEvent::OverlayChanged);
                    });
                }),
            )
        })
        .when(!is_busy(task), |this| {
            this.child(
                crate::widgets::IconButton::new(
                    format!("overlay-act-{task_id}-remove"),
                    AppIcon::Ui(UiIcon::DELETE),
                    rust_i18n::t!("sftp.overlay_remove"),
                )
                .size(crate::widgets::IconButtonSize::Compact)
                .render()
                .on_click(move |_, _, cx| {
                    view_remove.update(cx, |view, cx| {
                        view.remove_task(task_id, cx);
                        view.sync_drop_overlay(cx);
                        cx.emit(SftpPanelEvent::OverlayChanged);
                    });
                }),
            )
        });

    h_flex()
        .gap_1()
        .items_center()
        .child(AppIconElement::new(icon).xsmall().text_color(icon_color))
        .child(div().min_w_0().flex_1().text_xs().truncate().child(name))
        .child(bar)
        .child(
            div()
                .flex_none()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(status_text),
        )
        .child(actions)
        .into_any_element()
}
