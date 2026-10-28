//! 终端右下角的上传浮层：拖拽上传不再展开 SFTP 面板，进度改在这里承接。
//!
//! 设计原理（Why）：
//! - 实现初衷：拖文件的意图是"把这个丢到我现在这台机器上"，为此弹出一整列文件
//!   管理器面板会把注意力从终端拽走、还要手动关掉；浮层只回答"传了几个、多快、
//!   要不要停"，看完即走。
//! - 核心优势：队列仍是唯一事实源（`SftpPanelState::queue`），浮层只读它，动作
//!   直接调用面板那套现成的 pause/resume/cancel/remove，不存在第二套状态机；
//!   面板一旦展开，浮层立刻让位（`visible` 置假），不会出现两处进度各自刷新的漂移。
//! - 代价与局限：浮层挂在窗格（pane）的终端区右下角，一个窗格一份——队列本来就是
//!   窗格级的，分屏时每个窗格各显示自己的浮层；速率由 100ms 节流的进度事件做一阶
//!   平滑得出，暂停瞬间归零是事实而非误差，不做额外的"保持上次速率"美化。

use super::SftpPanelState;
use super::format_size;
use super::transfer_presentation::{display_name, is_busy, progress_percent, status_label};
use crate::app::{AppShell, TerminalTab};
use crate::palette::Palette;
use crate::status_bar::format_rate;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::progress::Progress;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use std::time::{Duration, Instant};
use transom_core::{TabId, TransferStatus, TransferTask};

/// 队列全部结束到浮层自动收起之间的停留时间。
const OVERLAY_LINGER: Duration = Duration::from_secs(5);
/// 明细区最大高度：几十个文件时不能把整个终端遮住。
const OVERLAY_MAX_HEIGHT: Pixels = px(240.0);
/// 浮层与窗格右下内边缘的距离，与终端搜索条的留白同量级。
const OVERLAY_INSET: Pixels = px(18.0);
/// 单行进度条宽度。
const OVERLAY_BAR_WIDTH: Pixels = px(96.0);

/// 浮层可见性与速率采样状态。
#[derive(Default)]
pub(crate) struct DropOverlayState {
    /// 是否显示。
    pub(crate) visible: bool,
    /// 是否展开明细行；收起时只保留一行汇总。
    pub(crate) expanded: bool,
    /// 队列排空后的收起任务；覆盖句柄即取消（dropping `Task` 会终止任务）。
    settle_task: Option<Task<()>>,
    /// 速率采样锚点：上一次进度事件的时刻与累计字节数。
    rate_anchor: Option<(Instant, u64)>,
    /// 一阶平滑后的字节/秒速率。
    rate_bytes_per_sec: u64,
}

impl DropOverlayState {
    /// 用聚合进度刷新速率。
    ///
    /// 并行传输时事件间隔可短到十几毫秒，单区间差值抖动很大，因此做一次一阶
    /// 低通（新值占四分之一权重）；累计字节回退（重排队、续传起点变化）用饱和
    /// 减法归到 0，绝不允许下溢。
    fn sample_rate(&mut self, done_bytes: u64) {
        let now = Instant::now();
        let Some((at, at_bytes)) = self.rate_anchor else {
            self.rate_anchor = Some((now, done_bytes));
            return;
        };
        let micros = now.duration_since(at).as_micros().max(1) as u64;
        let delta = done_bytes.saturating_sub(at_bytes);
        let instant = delta.saturating_mul(1_000_000) / micros;
        self.rate_bytes_per_sec = self
            .rate_bytes_per_sec
            .saturating_mul(3)
            .saturating_add(instant)
            / 4;
        self.rate_anchor = Some((now, done_bytes));
    }

    /// 传输停顿时速率归零，避免浮层上一直挂着最后一次的数字。
    fn reset_rate(&mut self) {
        self.rate_anchor = None;
        self.rate_bytes_per_sec = 0;
    }
}

impl AppShell {
    /// 按窗格 id 取其 SFTP 状态。
    ///
    /// 不复用 `sftp_panel_mut(tab_id, connection_id)`：浮层只关心队列，而队列状态
    /// 随连接建立而创建、断开即释放（见 `connection.rs` 与 `tabs.rs` 的清理点），
    /// 因此这里不需要再拿连接号复核一遍。
    fn panel_of(tabs: &mut [TerminalTab], tab_id: TabId) -> Option<&mut SftpPanelState> {
        tabs.iter_mut()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| tab.sftp_mut())
    }

    /// 拖入即显示：展开明细并撤掉待收起定时器。
    pub(crate) fn show_drop_overlay(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(panel) = Self::panel_of(&mut self.workspace.tabs, tab_id) else {
            return;
        };
        // 面板已经展开时不弹浮层：它的队列区就在旁边，两处同时刷进度是纯重复。
        if panel.open {
            return;
        }
        panel.queue.drop_overlay.visible = true;
        panel.queue.drop_overlay.expanded = true;
        panel.queue.drop_overlay.settle_task = None;
        cx.notify();
    }

    /// 队列状态变化后同步浮层：面板展开即退出，繁忙则常驻，排空则起停留定时器。
    pub(crate) fn sync_drop_overlay(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(panel) = Self::panel_of(&mut self.workspace.tabs, tab_id) else {
            return;
        };
        // 面板展开时由面板的队列区呈现进度，浮层不再重复。
        if panel.open {
            panel.queue.drop_overlay.visible = false;
            panel.queue.drop_overlay.settle_task = None;
            return;
        }
        if panel.queue.tasks.iter().any(is_busy) {
            panel.queue.drop_overlay.visible = true;
            panel.queue.drop_overlay.settle_task = None;
            return;
        }
        if !panel.queue.drop_overlay.visible || panel.queue.drop_overlay.settle_task.is_some() {
            return;
        }
        panel.queue.drop_overlay.reset_rate();
        let task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(OVERLAY_LINGER).await;
            let _ = this.update(cx, |this, cx| {
                let Some(panel) = Self::panel_of(&mut this.workspace.tabs, tab_id) else {
                    return;
                };
                // 停留期间若又忙起来，`sync_drop_overlay` 已把本句柄换成 None；
                // 能走到这里说明任务未被取消，直接收起。
                panel.queue.drop_overlay.visible = false;
                panel.queue.drop_overlay.expanded = true;
                panel.queue.drop_overlay.settle_task = None;
                cx.notify();
            });
        });
        panel.queue.drop_overlay.settle_task = Some(task);
    }

    /// 进度事件驱动的刷新：更新速率，并且只在窗格正被渲染时触发重绘。
    ///
    /// 进度事件是高频信号，后台标签的浮层不值得整帧重绘外壳。
    pub(crate) fn note_drop_overlay_progress(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if let Some(panel) = Self::panel_of(&mut self.workspace.tabs, tab_id)
            && !panel.open
            && panel.queue.drop_overlay.visible
        {
            let done_bytes = panel.queue.progress.map_or(0, |(done, _)| done);
            panel.queue.drop_overlay.sample_rate(done_bytes);
        }
        if self.is_rendered_pane(tab_id) {
            cx.notify();
        }
    }

    /// 用户点关闭：撤下浮层与定时器，下次拖入会重新出现。
    fn dismiss_drop_overlay(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if let Some(panel) = Self::panel_of(&mut self.workspace.tabs, tab_id) {
            panel.queue.drop_overlay.visible = false;
            panel.queue.drop_overlay.settle_task = None;
            panel.queue.drop_overlay.reset_rate();
        }
        cx.notify();
    }

    /// 渲染窗格终端区右下角的上传浮层；不该显示时返回 `None`。
    pub(crate) fn render_drop_overlay(
        &self,
        tab_id: TabId,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let panel = self
            .workspace
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)?
            .sftp()?;
        if panel.open || !panel.queue.drop_overlay.visible || panel.queue.tasks.is_empty() {
            return None;
        }
        let expanded = panel.queue.drop_overlay.expanded;
        let total = panel.queue.tasks.len();
        let busy = panel
            .queue
            .tasks
            .iter()
            .filter(|task| is_busy(task))
            .count();
        let speed = panel.queue.drop_overlay.rate_bytes_per_sec;
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

        let header = h_flex()
            .w_full()
            .items_center()
            .gap_1()
            .child(
                Icon::new(IconName::Upload)
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
                Button::new(format!("overlay-toggle-{tab_id}"))
                    .ghost()
                    .xsmall()
                    .icon(if expanded {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    })
                    .tooltip(rust_i18n::t!("sftp.overlay_expand"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let Some(panel) = Self::panel_of(&mut this.workspace.tabs, tab_id) else {
                            return;
                        };
                        panel.queue.drop_overlay.expanded = !panel.queue.drop_overlay.expanded;
                        cx.notify();
                    })),
            )
            .child(
                Button::new(format!("overlay-dismiss-{tab_id}"))
                    .ghost()
                    .xsmall()
                    .icon(IconName::X)
                    .tooltip(rust_i18n::t!("sftp.overlay_dismiss"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.dismiss_drop_overlay(tab_id, cx);
                    })),
            );

        let bulk_actions = h_flex().gap_0p5().when(busy > 0, |this| {
            this.child(
                Button::new(format!("overlay-pause-all-{tab_id}"))
                    .ghost()
                    .xsmall()
                    .icon(IconName::Pause)
                    .tooltip(rust_i18n::t!("sftp.overlay_pause_all"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.sftp_pause_all(tab_id, cx);
                        this.sync_drop_overlay(tab_id, cx);
                    })),
            )
            .child(
                Button::new(format!("overlay-cancel-all-{tab_id}"))
                    .ghost()
                    .xsmall()
                    .icon(IconName::Ban)
                    .tooltip(rust_i18n::t!("sftp.overlay_cancel_all"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.sftp_cancel_all(tab_id, cx);
                        this.sync_drop_overlay(tab_id, cx);
                    })),
            )
        });

        let detail = v_flex()
            .gap_1()
            .max_h(OVERLAY_MAX_HEIGHT)
            .overflow_y_scrollbar()
            .when(expanded, |this| {
                this.children(
                    panel
                        .queue
                        .tasks
                        .iter()
                        .map(|task| overlay_row(tab_id, task, p, cx)),
                )
            });

        // 绝对定位挂在窗格的终端容器上（见 workspace/panes.rs 的 .relative()），
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
                .rounded(p.radius.lg)
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
}

/// 一行的明细：文件名 + 进度条 + 状态文案 + 与该状态相称的动作。
///
/// 动作集合与面板队列区同口径（在途→暂停/取消，暂停→继续/取消，结束→移除），
/// 免得同一个任务在两处能做的事不一样。
fn overlay_row(
    tab_id: TabId,
    task: &TransferTask,
    p: &Palette,
    cx: &mut Context<AppShell>,
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
        TransferStatus::Failed(reason) => reason.clone(),
        _ => status_label(&task.status),
    };
    let (icon, icon_color) = match &task.status {
        TransferStatus::Failed(_) => (IconName::CircleAlert, p.danger),
        TransferStatus::Cancelled => (IconName::CircleSlash, p.muted_foreground),
        TransferStatus::Completed => (IconName::Check, p.success),
        TransferStatus::Paused => (IconName::Pause, p.warning),
        _ => (IconName::Upload, p.sidebar_accent_foreground),
    };
    let bar = match &task.status {
        TransferStatus::Pending | TransferStatus::Transferring | TransferStatus::Paused => {
            Progress::new(format!("overlay-progress-{task_id}"))
                .w(OVERLAY_BAR_WIDTH)
                .rounded_full()
                .value(percent.unwrap_or(0.0))
                .into_any_element()
        }
        // 结束态没有进度可显示，用等宽占位保持各行右侧对齐。
        _ => div().w(OVERLAY_BAR_WIDTH).into_any_element(),
    };
    let paused = task.status == TransferStatus::Paused;

    let actions = h_flex()
        .gap_0p5()
        .when(is_busy(task), |this| {
            this.child(
                Button::new(format!("overlay-act-{task_id}-toggle"))
                    .ghost()
                    .xsmall()
                    .icon(if paused {
                        IconName::Play
                    } else {
                        IconName::Pause
                    })
                    .tooltip(rust_i18n::t!(if paused {
                        "sftp.overlay_resume"
                    } else {
                        "sftp.overlay_pause"
                    }))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if paused {
                            this.sftp_resume_task(tab_id, task_id, cx);
                        } else {
                            this.sftp_pause_task(tab_id, task_id, cx);
                        }
                        this.sync_drop_overlay(tab_id, cx);
                    })),
            )
            .child(
                Button::new(format!("overlay-act-{task_id}-cancel"))
                    .ghost()
                    .xsmall()
                    .icon(IconName::X)
                    .tooltip(rust_i18n::t!("sftp.overlay_cancel"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.sftp_cancel_task(tab_id, task_id, cx);
                        this.sync_drop_overlay(tab_id, cx);
                    })),
            )
        })
        .when(!is_busy(task), |this| {
            this.child(
                Button::new(format!("overlay-act-{task_id}-remove"))
                    .ghost()
                    .xsmall()
                    .icon(IconName::Trash)
                    .tooltip(rust_i18n::t!("sftp.overlay_remove"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.sftp_remove_task(tab_id, task_id, cx);
                        this.sync_drop_overlay(tab_id, cx);
                    })),
            )
        });

    h_flex()
        .gap_1()
        .items_center()
        .child(Icon::new(icon).xsmall().text_color(icon_color))
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
