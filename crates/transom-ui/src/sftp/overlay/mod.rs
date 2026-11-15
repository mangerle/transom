//! 终端右下角的上传浮层：拖拽上传不再展开 SFTP 面板，进度改在这里承接。
//!
//! 设计原理（Why）：
//! - 实现初衷：拖文件的意图是"把这个丢到我现在这台机器上"，为此弹出一整列文件
//!   管理器面板会把注意力从终端拽走、还要手动关掉；浮层只回答"传了几个、多快、
//!   要不要停"，看完即走。
//! - 核心优势：队列仍是唯一事实源（`SftpPanelState::queue`），浮层状态与定时器均
//!   归属 `SftpPanelView` 实体自身，彻底解除对主壳层的依赖；通过
//!   `SftpPanelEvent::OverlayChanged` 通知外壳重绘，面板展开时立刻让位。
//! - 代价与局限：速率由 100ms 节流的进度事件做一阶平滑得出，暂停瞬间归零是事实而非误差。

mod render;

use super::SftpPanelEvent;
use super::entity::SftpPanelView;
use super::transfer_presentation::is_busy;
use gpui_kit::*;
use std::time::{Duration, Instant};

pub(crate) use render::render_drop_overlay;

/// 队列全部结束到浮层自动收起之间的停留时间。
const OVERLAY_LINGER: Duration = Duration::from_secs(5);

/// 浮层可见性与速率采样状态。
#[derive(Default)]
pub(crate) struct DropOverlayState {
    /// 是否显示。
    pub(crate) visible: bool,
    /// 是否展开明细行；收起时只保留一行汇总。
    pub(crate) expanded: bool,
    /// 队列排空后的收起任务；覆盖句柄即取消（dropping `Task` 会终止任务）。
    pub(super) settle_task: Option<Task<()>>,
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

    /// 当前平滑后的传输速率（字节/秒）。
    pub(super) fn rate_bytes_per_sec(&self) -> u64 {
        self.rate_bytes_per_sec
    }
}

impl SftpPanelView {
    /// 队列状态变化后同步浮层：面板展开即退出，繁忙则常驻，排空则起停留定时器。
    pub(crate) fn sync_drop_overlay(&mut self, cx: &mut Context<Self>) {
        // 面板展开时由面板的队列区呈现进度，浮层不再重复。
        if self.state.open {
            self.state.queue.drop_overlay.visible = false;
            self.state.queue.drop_overlay.settle_task = None;
            return;
        }
        let busy = self.state.queue.tasks.iter().any(is_busy);
        if busy {
            self.state.queue.drop_overlay.visible = true;
            self.state.queue.drop_overlay.settle_task = None;
            return;
        }
        if !self.state.queue.drop_overlay.visible
            || self.state.queue.drop_overlay.settle_task.is_some()
        {
            return;
        }
        let task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(OVERLAY_LINGER).await;
            let _ = this.update(cx, |view, cx| {
                // 停留期间若又忙起来，`sync_drop_overlay` 已把本句柄换成 None；
                // 能走到这里说明任务未被取消，直接收起。
                view.state.queue.drop_overlay.visible = false;
                view.state.queue.drop_overlay.expanded = true;
                view.state.queue.drop_overlay.settle_task = None;
                cx.emit(SftpPanelEvent::OverlayChanged);
                cx.notify();
            });
        });
        self.state.queue.drop_overlay.reset_rate();
        self.state.queue.drop_overlay.settle_task = Some(task);
    }

    /// 进度事件驱动的速率采样：仅在面板收起且浮层可见时计算平滑速率。
    pub(crate) fn note_drop_overlay_progress(&mut self) {
        if !self.state.open && self.state.queue.drop_overlay.visible {
            let done_bytes = self.state.queue.progress.map_or(0, |(done, _)| done);
            self.state.queue.drop_overlay.sample_rate(done_bytes);
        }
    }

    /// 切换浮层明细区的展开/收起状态。
    pub(super) fn toggle_drop_overlay_expanded(&mut self, cx: &mut Context<Self>) {
        self.state.queue.drop_overlay.expanded = !self.state.queue.drop_overlay.expanded;
        cx.emit(SftpPanelEvent::OverlayChanged);
        cx.notify();
    }

    /// 用户点关闭：撤下浮层与定时器，下次拖入会重新出现。
    pub(super) fn dismiss_drop_overlay(&mut self, cx: &mut Context<Self>) {
        self.state.queue.drop_overlay.visible = false;
        self.state.queue.drop_overlay.settle_task = None;
        self.state.queue.drop_overlay.reset_rate();
        cx.emit(SftpPanelEvent::OverlayChanged);
        cx.notify();
    }
}
