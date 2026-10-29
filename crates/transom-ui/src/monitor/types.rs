//! 主机监控状态模型与辅助计算工具。

use crate::app::AppShell;
use gpui_kit::base::VirtualListScrollHandle;
use gpui_kit::component::input::InputState;
use gpui_kit::*;
use std::collections::VecDeque;
use std::time::Duration;
use transom_core::{ProcessInfo, TabId};

pub(crate) use transom_core::ProcessSort;

/// 历史波形保留的采样点数：约两分钟历史。
pub(crate) const HISTORY_POINTS: usize = 40;

/// 采样周期档位。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum SampleIntervalMode {
    /// 极速排查模式 (1秒)
    Fast,
    /// 默认平稳模式 (3秒)
    #[default]
    Normal,
    /// 节能降噪模式 (10秒)
    Eco,
}

impl SampleIntervalMode {
    pub(crate) fn interval(&self) -> Duration {
        match self {
            Self::Fast => Duration::from_secs(1),
            Self::Normal => Duration::from_secs(3),
            Self::Eco => Duration::from_secs(10),
        }
    }

    pub(crate) fn label(&self) -> &'static str {
        match self {
            Self::Fast => "1s",
            Self::Normal => "3s",
            Self::Eco => "10s",
        }
    }
}

/// 各板块折叠/展开状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MonitorSectionCollapse {
    pub(crate) cpu: bool,
    pub(crate) memory: bool,
    pub(crate) disk: bool,
    pub(crate) network: bool,
    pub(crate) ports: bool,
    pub(crate) services: bool,
    pub(crate) processes: bool,
}

impl Default for MonitorSectionCollapse {
    fn default() -> Self {
        Self {
            cpu: false,
            memory: false,
            disk: true,
            network: false,
            ports: true,
            services: true,
            processes: false,
        }
    }
}

impl From<&transom_core::MonitorSettings> for MonitorSectionCollapse {
    fn from(s: &transom_core::MonitorSettings) -> Self {
        Self {
            cpu: !s.cpu_expanded,
            memory: !s.memory_expanded,
            disk: !s.disk_expanded,
            network: !s.network_expanded,
            ports: !s.ports_expanded,
            services: !s.services_expanded,
            processes: !s.processes_expanded,
        }
    }
}

/// 监控走势图渲染形态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum SparklineChartType {
    /// 平滑折线面积图（默认）。
    #[default]
    Line,
    /// 紧凑柱状图。
    Bar,
}

/// 监控看板的界面状态。
pub(crate) struct MonitorState {
    pub(crate) open: bool,
    /// 抽屉当前宽度（像素）。
    pub(crate) drawer_width: f32,
    /// 板块折叠状态。
    pub(crate) collapse: MonitorSectionCollapse,
    /// 采样频率档位。
    pub(crate) interval_mode: SampleIntervalMode,
    /// 走势图渲染形态。
    pub(crate) chart_type: SparklineChartType,
    /// 进程排序依据。
    pub(crate) sort: ProcessSort,
    /// 进程排序方向：false 为降序（最大值在前），点同一列表头翻转。
    pub(crate) sort_ascending: bool,
    /// 进程明细快照。
    pub(crate) processes: Vec<ProcessInfo>,
    /// 进程列表滚动句柄：虚拟列表与可见滚动条共用同一个偏移。
    pub(crate) process_scroll: VirtualListScrollHandle,
    /// 监听端口列表滚动句柄。
    pub(crate) ports_scroll: VirtualListScrollHandle,
    /// 主机进程总数：折叠态由轻量计数采样刷新，展开态等于明细条数。
    pub(crate) process_total: u32,
    /// 当前选中查看完整命令行的进程 PID。
    pub(crate) selected_process_pid: Option<u32>,
    pub(crate) error: Option<String>,
    pub(crate) filter: Entity<InputState>,
    /// 监听端口筛选框：端口表按 5 秒一轮刷新，条目常常超过一屏，
    /// 与进程筛选分开持有，避免两个列表共用一个词互相把对方的结果筛空。
    pub(crate) ports_filter: Entity<InputState>,
    /// CPU 历史走势（0.0 ~ 100.0）。
    pub(crate) cpu_history: VecDeque<f32>,
    /// 内存使用率历史走势（0.0 ~ 100.0）。
    pub(crate) mem_history: VecDeque<f32>,
    /// 网络下行速率历史（字节/秒）。
    pub(crate) rx_history: VecDeque<u64>,
    /// 网络上行速率历史（字节/秒）。
    pub(crate) tx_history: VecDeque<u64>,
    /// 波形归属的标签；切换标签时清空，避免把上一台机器的曲线画到新主机上。
    pub(crate) history_tab: Option<TabId>,
}

impl MonitorState {
    pub(crate) fn new(
        window: &mut Window,
        settings: &transom_core::MonitorSettings,
        cx: &mut Context<AppShell>,
    ) -> Self {
        Self {
            open: false,
            drawer_width: settings.drawer_width.clamp(360.0, 750.0),
            collapse: MonitorSectionCollapse::from(settings),
            interval_mode: SampleIntervalMode::Normal,
            chart_type: SparklineChartType::default(),
            sort: ProcessSort::Cpu,
            sort_ascending: false,
            processes: Vec::new(),
            process_scroll: VirtualListScrollHandle::new(),
            ports_scroll: VirtualListScrollHandle::new(),
            process_total: 0,
            selected_process_pid: None,
            error: None,
            filter: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(rust_i18n::t!("monitor.filter_placeholder").to_string())
            }),
            ports_filter: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(rust_i18n::t!("monitor.ports_filter_placeholder").to_string())
            }),
            cpu_history: VecDeque::with_capacity(HISTORY_POINTS),
            mem_history: VecDeque::with_capacity(HISTORY_POINTS),
            rx_history: VecDeque::with_capacity(HISTORY_POINTS),
            tx_history: VecDeque::with_capacity(HISTORY_POINTS),
            history_tab: None,
        }
    }

    /// 记录一轮统计数据，更新时序队列。
    pub(crate) fn record_stats(&mut self, tab_id: TabId, stats: &transom_core::SystemStats) {
        if self.history_tab != Some(tab_id) {
            self.history_tab = Some(tab_id);
            self.cpu_history.clear();
            self.mem_history.clear();
            self.rx_history.clear();
            self.tx_history.clear();
        }

        // 记录 CPU
        if self.cpu_history.len() >= HISTORY_POINTS {
            self.cpu_history.pop_front();
        }
        self.cpu_history.push_back(stats.cpu_percent);

        // 记录内存
        let mem_pct = if stats.mem_total_bytes > 0 {
            (stats.mem_used_bytes as f32 / stats.mem_total_bytes as f32) * 100.0
        } else {
            0.0
        };
        if self.mem_history.len() >= HISTORY_POINTS {
            self.mem_history.pop_front();
        }
        self.mem_history.push_back(mem_pct);

        // 记录总网络下行与上行
        let (total_rx, total_tx) = stats
            .interfaces
            .iter()
            .fold((0u64, 0u64), |(rx, tx), iface| {
                (
                    rx.saturating_add(iface.rx_bytes_per_sec),
                    tx.saturating_add(iface.tx_bytes_per_sec),
                )
            });
        if self.rx_history.len() >= HISTORY_POINTS {
            self.rx_history.pop_front();
        }
        self.rx_history.push_back(total_rx);
        if self.tx_history.len() >= HISTORY_POINTS {
            self.tx_history.pop_front();
        }
        self.tx_history.push_back(total_tx);
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
