//! 主机监控状态模型与辅助计算工具。

use crate::app::AppShell;
use gpui_kit::component::input::InputState;
use gpui_kit::*;
use std::collections::VecDeque;
use std::time::Duration;
use transom_core::{InterfaceRate, ProcessInfo, TabId};

/// 网卡波形保留的采样点数：3 秒一个点，约两分钟历史。
pub(crate) const NETWORK_HISTORY: usize = 40;
/// 进程面板轮询周期，与系统指标采样保持一致。
pub(crate) const PROCESS_POLL_INTERVAL: Duration = Duration::from_secs(3);

/// 进程列表排序键。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProcessSort {
    Cpu,
    Memory,
}

/// 监控看板的界面状态。
pub(crate) struct MonitorState {
    pub(crate) open: bool,
    pub(crate) processes: Vec<ProcessInfo>,
    pub(crate) error: Option<String>,
    pub(crate) sort: ProcessSort,
    pub(crate) filter: Entity<InputState>,
    /// 每轮网卡快照，用于绘制下行速率波形。
    pub(crate) network_history: VecDeque<Vec<InterfaceRate>>,
    /// 波形归属的标签；切换标签时清空，避免把上一台机器的曲线画到新主机上。
    pub(crate) history_tab: Option<TabId>,
}

impl MonitorState {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<AppShell>) -> Self {
        Self {
            open: false,
            processes: Vec::new(),
            error: None,
            sort: ProcessSort::Cpu,
            filter: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(rust_i18n::t!("monitor.filter_placeholder").to_string())
            }),
            network_history: VecDeque::with_capacity(NETWORK_HISTORY),
            history_tab: None,
        }
    }

    pub(crate) fn record_network(&mut self, tab_id: TabId, interfaces: &[InterfaceRate]) {
        if self.history_tab != Some(tab_id) {
            self.history_tab = Some(tab_id);
            self.network_history.clear();
        }
        if interfaces.is_empty() {
            return;
        }
        if self.network_history.len() >= NETWORK_HISTORY {
            self.network_history.pop_front();
        }
        self.network_history.push_back(interfaces.to_vec());
    }
}

pub(crate) fn usage_bar(fraction: f32, color: Hsla, p: &crate::palette::Palette) -> AnyElement {
    let fraction = fraction.clamp(0.0, 1.0);
    div()
        .h(px(6.0))
        .w_full()
        .rounded_full()
        .bg(p.muted)
        .child(
            div()
                .h_full()
                .w(relative(fraction))
                .rounded_full()
                .bg(color),
        )
        .into_any_element()
}

pub(crate) fn usage_color(percent: f32, p: &crate::palette::Palette) -> Hsla {
    if percent > 85.0 {
        p.danger
    } else if percent > 65.0 {
        p.sidebar_accent_foreground
    } else {
        p.primary
    }
}

pub(crate) use crate::format::format_compact_size as format_size;
