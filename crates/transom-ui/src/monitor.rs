//! 主机监控看板与进程管理器：Agentless，数据来自现有 SSH 采样与按需 exec。
//!
//! 设计原理（Why）：
//! - 指标在状态栏采样器里已经每 3 秒更新一次，看板直接消费同一份
//!   `SystemStats`，不再为面板单独开一条采集链路；
//! - 进程列表是"按需"数据：面板打开时才拉取，关闭即停止，避免常驻远端
//!   跑 `ps` 浪费对方 CPU；
//! - 信号操作只放行 SIGTERM/SIGKILL 且拒绝 PID 1，界面层与执行层各拦一道。
//!
//! 代价与局限：网卡波形只保留最近 40 个采样点（约 2 分钟），不落盘；
//! 进程列表用 `comm` 短名而非完整命令行，避免每轮为 50 个进程开 50 次 exec。

use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{Selectable, h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::menu::{ContextMenuExt, PopupMenuItem};
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;
use std::collections::VecDeque;
use std::time::Duration;
use transom_core::{
    AppCommand, AppError, ConnectionId, InterfaceRate, ProcessInfo, SystemStats, TabId,
};

/// 网卡波形保留的采样点数：3 秒一个点，约两分钟历史。
const NETWORK_HISTORY: usize = 40;
/// 进程面板轮询周期，与系统指标采样保持一致。
const PROCESS_POLL_INTERVAL: Duration = Duration::from_secs(3);

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
    network_history: VecDeque<Vec<InterfaceRate>>,
    /// 波形归属的标签；切换标签时清空，避免把上一台机器的曲线画到新主机上。
    history_tab: Option<TabId>,
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

    fn record_network(&mut self, tab_id: TabId, interfaces: &[InterfaceRate]) {
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

impl AppShell {
    /// 活动栏入口：打开/关闭监控抽屉，打开时立即拉取一轮进程快照。
    pub(crate) fn toggle_monitor(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.monitor.open = !self.monitor.open;
        if self.monitor.open {
            self.monitor.history_tab = None;
            self.monitor.error = None;
            self.sample_processes(cx);
            self.ensure_monitor_task(cx);
        } else {
            self.monitor.processes.clear();
        }
        cx.notify();
    }

    /// 保证进程轮询任务已启动；任务常驻但在面板关闭时不发命令。
    fn ensure_monitor_task(&mut self, cx: &mut Context<Self>) {
        if self.monitor_task.is_some() {
            return;
        }
        self.monitor_task = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(PROCESS_POLL_INTERVAL).await;
                if this
                    .update(cx, |this, cx| this.sample_processes_if_open(cx))
                    .is_err()
                {
                    break;
                }
            }
        }));
    }

    fn sample_processes_if_open(&mut self, cx: &mut Context<Self>) {
        if !self.monitor.open {
            return;
        }
        self.sample_processes(cx);
    }

    fn sample_processes(&mut self, cx: &mut Context<Self>) {
        let Some((tab_id, connection_id)) = self.active_monitor_target() else {
            return;
        };
        self.send_command(AppCommand::SampleProcesses {
            tab_id,
            connection_id,
        });
        cx.notify();
    }

    /// 当前活动标签的进程采样目标；只有 SSH 与本地终端支持。
    fn active_monitor_target(&self) -> Option<(TabId, ConnectionId)> {
        let tab_id = self.active_tab?;
        let tab = self.tabs.iter().find(|tab| tab.id == tab_id)?;
        if !matches!(
            tab.session.connection,
            transom_core::Connection::Ssh(_) | transom_core::Connection::Local(_)
        ) {
            return None;
        }
        Some((tab_id, tab.connection_id?))
    }

    fn send_kill_process(&mut self, pid: u32, force: bool, cx: &mut Context<Self>) {
        let Some((tab_id, connection_id)) = self.active_monitor_target() else {
            return;
        };
        self.send_command(AppCommand::KillProcess {
            tab_id,
            connection_id,
            pid,
            force,
        });
        cx.notify();
    }

    fn on_processes_sampled(
        &mut self,
        tab_id: TabId,
        connection_id: ConnectionId,
        processes: Vec<ProcessInfo>,
        cx: &mut Context<Self>,
    ) {
        if self.active_monitor_target() != Some((tab_id, connection_id)) {
            return;
        }
        self.monitor.processes = processes;
        self.monitor.error = None;
        cx.notify();
    }

    fn on_processes_failed(
        &mut self,
        tab_id: TabId,
        connection_id: ConnectionId,
        error: AppError,
        cx: &mut Context<Self>,
    ) {
        if self.active_monitor_target() != Some((tab_id, connection_id)) {
            return;
        }
        self.monitor.error = Some(crate::palette::localized_error(&error));
        cx.notify();
    }

    fn on_process_killed(
        &mut self,
        tab_id: TabId,
        connection_id: ConnectionId,
        pid: u32,
        error: Option<AppError>,
        cx: &mut Context<Self>,
    ) {
        if self.active_monitor_target() != Some((tab_id, connection_id)) {
            return;
        }
        match error {
            Some(error) => {
                self.monitor.error = Some(crate::palette::localized_error(&error));
            }
            None => {
                self.monitor.error = None;
                self.status = Some(rust_i18n::t!("monitor.kill_sent", pid = pid).to_string());
                // 立即拉一轮，让用户看到进程确实消失（或变成僵尸）。
                self.sample_processes(cx);
            }
        }
        cx.notify();
    }

    /// 渲染右侧监控抽屉。
    pub(crate) fn render_monitor_panel(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let stats = self
            .active_tab
            .and_then(|tab_id| self.tabs.iter().find(|tab| tab.id == tab_id))
            .and_then(|tab| tab.system_stats.clone());
        let mut body = v_flex().gap_4().p_3();
        if let Some(stats) = &stats {
            body = body
                .child(self.render_cpu_section(stats, p))
                .child(self.render_memory_section(stats, p))
                .child(self.render_filesystems_section(stats, p))
                .child(self.render_network_section(p));
        } else {
            body = body.child(
                div()
                    .text_sm()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("monitor.waiting").to_string()),
            );
        }
        body = body.child(self.render_process_section(cx, p));

        let mut panel = v_flex()
            .id("monitor-panel")
            .size_full()
            .bg(p.background)
            .border_l_1()
            .border_color(p.border);
        panel = panel.child(
            h_flex()
                .justify_between()
                .items_center()
                .px_3()
                .py_2()
                .border_b_1()
                .border_color(p.border)
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(Icon::new(IconName::Gauge).small().text_color(p.primary))
                        .child(
                            div()
                                .text_sm()
                                .child(rust_i18n::t!("monitor.title").to_string()),
                        ),
                )
                .child(
                    h_flex()
                        .gap_1()
                        .child(
                            Button::new("monitor-refresh")
                                .ghost()
                                .small()
                                .icon(IconName::RefreshCw)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.sample_processes(cx);
                                })),
                        )
                        .child(
                            Button::new("monitor-close")
                                .ghost()
                                .small()
                                .icon(IconName::Close)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.monitor.open = false;
                                    this.monitor.processes.clear();
                                    cx.notify();
                                })),
                        ),
                ),
        );
        if let Some(error) = &self.monitor.error {
            panel = panel.child(
                div()
                    .px_3()
                    .py_2()
                    .text_xs()
                    .text_color(p.danger)
                    .child(error.clone()),
            );
        }
        panel
            .child(
                div()
                    .id("monitor-body")
                    .flex_1()
                    .overflow_y_scroll()
                    .child(body),
            )
            .into_any_element()
    }

    fn render_cpu_section(&self, stats: &SystemStats, p: &Palette) -> AnyElement {
        let color = usage_color(stats.cpu_percent, p);
        let mut section = v_flex()
            .gap_2()
            .child(self.metric_header(
                "monitor.cpu",
                format!("{:.0}%", stats.cpu_percent),
                color,
                p,
            ))
            .child(usage_bar(stats.cpu_percent / 100.0, color, p));
        if !stats.cpu_cores.is_empty() {
            section = section.child(h_flex().h(px(42.0)).items_end().gap_0p5().children(
                stats.cpu_cores.iter().map(|core| {
                    let fraction = (core.percent / 100.0).clamp(0.0, 1.0);
                    div()
                        .flex_1()
                        .h(px((42.0 * fraction).max(2.0)))
                        .rounded_t_sm()
                        .bg(usage_color(core.percent, p))
                }),
            ));
        }
        section.into_any_element()
    }

    fn render_memory_section(&self, stats: &SystemStats, p: &Palette) -> AnyElement {
        let fraction = if stats.mem_total_bytes == 0 {
            0.0
        } else {
            stats.mem_used_bytes as f32 / stats.mem_total_bytes as f32
        };
        v_flex()
            .gap_2()
            .child(self.metric_header(
                "monitor.mem",
                format!(
                    "{} / {}",
                    format_size(stats.mem_used_bytes),
                    format_size(stats.mem_total_bytes)
                ),
                p.primary,
                p,
            ))
            .child(usage_bar(fraction, p.primary, p))
            .into_any_element()
    }

    fn render_filesystems_section(&self, stats: &SystemStats, p: &Palette) -> AnyElement {
        let mut section = v_flex()
            .gap_2()
            .child(self.section_title("monitor.disk", p));
        if stats.filesystems.is_empty() {
            section = section.child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("monitor.no_data").to_string()),
            );
        }
        for filesystem in stats.filesystems.iter().take(8) {
            let fraction = if filesystem.total_bytes == 0 {
                0.0
            } else {
                filesystem.used_bytes as f32 / filesystem.total_bytes as f32
            };
            let color = usage_color(fraction * 100.0, p);
            section = section.child(
                v_flex()
                    .gap_1()
                    .child(
                        h_flex()
                            .justify_between()
                            .text_xs()
                            .child(
                                div()
                                    .text_color(p.foreground)
                                    .child(filesystem.mount.clone()),
                            )
                            .child(div().text_color(p.muted_foreground).child(format!(
                                "{} / {}",
                                format_size(filesystem.used_bytes),
                                format_size(filesystem.total_bytes)
                            ))),
                    )
                    .child(usage_bar(fraction, color, p)),
            );
        }
        section.into_any_element()
    }

    fn render_network_section(&self, p: &Palette) -> AnyElement {
        let mut section = v_flex()
            .gap_2()
            .child(self.section_title("monitor.network", p));
        let latest = self
            .monitor
            .network_history
            .back()
            .cloned()
            .unwrap_or_default();
        if latest.is_empty() {
            section = section.child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("monitor.no_data").to_string()),
            );
        }
        for interface in &latest {
            let history: Vec<u64> = self
                .monitor
                .network_history
                .iter()
                .filter_map(|sample| {
                    sample
                        .iter()
                        .find(|entry| entry.name == interface.name)
                        .map(|entry| entry.rx_bytes_per_sec)
                })
                .collect();
            let peak = history.iter().copied().max().unwrap_or(1).max(1);
            section = section.child(
                v_flex()
                    .gap_1()
                    .child(
                        h_flex()
                            .justify_between()
                            .text_xs()
                            .child(div().text_color(p.foreground).child(interface.name.clone()))
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(div().text_color(p.muted_foreground).child(format!(
                                        "↓ {}/s",
                                        format_size(interface.rx_bytes_per_sec)
                                    )))
                                    .child(div().text_color(p.muted_foreground).child(format!(
                                        "↑ {}/s",
                                        format_size(interface.tx_bytes_per_sec)
                                    ))),
                            ),
                    )
                    .child(h_flex().h(px(22.0)).items_end().gap_0p5().children(
                        history.iter().map(|value| {
                            let fraction = (*value as f32 / peak as f32).clamp(0.0, 1.0);
                            div()
                                .w(px(3.0))
                                .h(px((22.0 * fraction).max(1.0)))
                                .bg(p.primary.opacity(0.65))
                        }),
                    )),
            );
        }
        section.into_any_element()
    }

    fn render_process_section(&mut self, cx: &mut Context<Self>, p: &Palette) -> AnyElement {
        let filter = self.monitor.filter.read(cx).value().to_lowercase();
        let mut processes: Vec<&ProcessInfo> = self
            .monitor
            .processes
            .iter()
            .filter(|process| {
                filter.is_empty()
                    || process.cmdline.to_lowercase().contains(&filter)
                    || process.user.to_lowercase().contains(&filter)
                    || process.pid.to_string().contains(&filter)
            })
            .collect();
        match self.monitor.sort {
            ProcessSort::Cpu => processes.sort_by(|a, b| {
                b.cpu_percent
                    .partial_cmp(&a.cpu_percent)
                    .unwrap_or(std::cmp::Ordering::Equal)
            }),
            ProcessSort::Memory => processes.sort_by(|a, b| {
                b.mem_percent
                    .partial_cmp(&a.mem_percent)
                    .unwrap_or(std::cmp::Ordering::Equal)
            }),
        }
        let sort = self.monitor.sort;
        let app_entity = cx.entity();
        let mut section = v_flex()
            .gap_2()
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(self.section_title("monitor.processes", p))
                            .child(
                                div().text_xs().text_color(p.muted_foreground).child(
                                    rust_i18n::t!("monitor.process_count", count = processes.len())
                                        .to_string(),
                                ),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .child(
                                Button::new("sort-process-cpu")
                                    .small()
                                    .label(rust_i18n::t!("monitor.sort_cpu").to_string())
                                    .selected(sort == ProcessSort::Cpu)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.monitor.sort = ProcessSort::Cpu;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("sort-process-mem")
                                    .small()
                                    .label(rust_i18n::t!("monitor.sort_mem").to_string())
                                    .selected(sort == ProcessSort::Memory)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.monitor.sort = ProcessSort::Memory;
                                        cx.notify();
                                    })),
                            ),
                    ),
            )
            .child(Input::new(&self.monitor.filter).small())
            .child(
                h_flex()
                    .gap_2()
                    .px_1()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(div().w(px(54.0)).child("PID"))
                    .child(
                        div()
                            .w(px(64.0))
                            .child(rust_i18n::t!("monitor.user").to_string()),
                    )
                    .child(div().w(px(48.0)).child("CPU"))
                    .child(div().w(px(48.0)).child("MEM"))
                    .child(
                        div()
                            .flex_1()
                            .child(rust_i18n::t!("monitor.command").to_string()),
                    ),
            );
        for (index, process) in processes.iter().enumerate() {
            let pid = process.pid;
            let command = format!("{} {}", process.pid, process.cmdline);
            let app = app_entity.clone();
            let strong = app_entity.clone();
            let mut row = h_flex()
                .id(ElementId::Name(format!("process-row-{pid}").into()))
                .gap_2()
                .px_1()
                .h(px(22.0))
                .text_xs()
                .child(
                    div()
                        .w(px(54.0))
                        .text_color(p.muted_foreground)
                        .child(pid.to_string()),
                )
                .child(
                    div()
                        .w(px(64.0))
                        .truncate()
                        .text_color(p.foreground)
                        .child(process.user.clone()),
                )
                .child(
                    div()
                        .w(px(48.0))
                        .text_color(usage_color(process.cpu_percent, p))
                        .child(format!("{:.1}", process.cpu_percent)),
                )
                .child(
                    div()
                        .w(px(48.0))
                        .text_color(p.muted_foreground)
                        .child(format!("{:.1}", process.mem_percent)),
                )
                .child(
                    div()
                        .flex_1()
                        .truncate()
                        .text_color(p.foreground)
                        .child(process.cmdline.clone()),
                )
                .context_menu(move |menu, _window, _cx| {
                    let term = app.clone();
                    let kill = strong.clone();
                    let command_text = command.clone();
                    menu.item(
                        PopupMenuItem::new(rust_i18n::t!("monitor.kill_term").to_string())
                            .icon(IconName::CircleX)
                            .on_click(move |_, _, cx| {
                                term.update(cx, |this, cx| this.send_kill_process(pid, false, cx));
                            }),
                    )
                    .item(
                        PopupMenuItem::new(rust_i18n::t!("monitor.kill_kill").to_string())
                            .icon(IconName::Ban)
                            .on_click(move |_, _, cx| {
                                kill.update(cx, |this, cx| this.send_kill_process(pid, true, cx));
                            }),
                    )
                    .item(
                        PopupMenuItem::new(rust_i18n::t!("monitor.copy_command").to_string())
                            .icon(IconName::Copy)
                            .on_click(move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(
                                    command_text.clone(),
                                ));
                            }),
                    )
                });
            if index % 2 == 1 {
                row = row.bg(p.muted.opacity(0.35));
            }
            section = section.child(row);
        }
        section.into_any_element()
    }

    fn section_title(&self, key: &'static str, p: &Palette) -> AnyElement {
        div()
            .text_xs()
            .text_color(p.muted_foreground)
            .child(rust_i18n::t!(key).to_string())
            .into_any_element()
    }

    fn metric_header(
        &self,
        key: &'static str,
        value: String,
        color: Hsla,
        p: &Palette,
    ) -> AnyElement {
        h_flex()
            .justify_between()
            .text_xs()
            .child(
                div()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!(key).to_string()),
            )
            .child(div().text_color(color).child(value))
            .into_any_element()
    }
}

fn usage_bar(fraction: f32, color: Hsla, p: &Palette) -> AnyElement {
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

fn usage_color(percent: f32, p: &Palette) -> Hsla {
    if percent > 85.0 {
        p.danger
    } else if percent > 65.0 {
        p.sidebar_accent_foreground
    } else {
        p.primary
    }
}

fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "K", "M", "G", "T"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes}B")
    } else {
        format!("{value:.1}{}", UNITS[unit])
    }
}

/// 供事件处理复用的看板状态更新入口。
impl AppShell {
    pub(crate) fn monitor_record_network(&mut self, tab_id: TabId, interfaces: &[InterfaceRate]) {
        if self.monitor.open {
            self.monitor.record_network(tab_id, interfaces);
        }
    }

    pub(crate) fn monitor_sampled(
        &mut self,
        tab_id: TabId,
        connection_id: ConnectionId,
        processes: Vec<ProcessInfo>,
        cx: &mut Context<Self>,
    ) {
        self.on_processes_sampled(tab_id, connection_id, processes, cx);
    }

    pub(crate) fn monitor_failed(
        &mut self,
        tab_id: TabId,
        connection_id: ConnectionId,
        error: AppError,
        cx: &mut Context<Self>,
    ) {
        self.on_processes_failed(tab_id, connection_id, error, cx);
    }

    pub(crate) fn monitor_killed(
        &mut self,
        tab_id: TabId,
        connection_id: ConnectionId,
        pid: u32,
        error: Option<AppError>,
        cx: &mut Context<Self>,
    ) {
        self.on_process_killed(tab_id, connection_id, pid, error, cx);
    }
}
