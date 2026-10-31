//! 监控面板实体：拥有监控状态与全部渲染，通过弱引用回到外壳执行跨域动作。
//!
//! # 设计原理（Why）
//! - 监控是窗口级面板，生命周期与窗口一致，因此独立成 `Entity<MonitorView>`，
//!   而不是把状态平铺在外壳里。这样统计/进程样本刷新只通知本实体，外壳不再
//!   因为一次采样而整棵树重建；
//! - 面板需要读取活动标签的统计数据，也需要触发采样、激活标签等跨域动作。前者
//!   通过 `WeakEntity<AppShell>` 只读获取，后者回交外壳执行；
//! - 用弱引用而非强引用：外壳强持有本实体，反向强引用会成环导致实体永不释放。
//!
//! # 代价与局限
//! 跨域动作多一次 `Entity::update` 调用；`app` 在构造时尚未创建完成，因此采用
//! 两段式绑定（`new` 之后立即 `attach`），绑定前跨域动作静默跳过。

use super::types::MonitorState;
use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::*;
use transom_core::{MonitorSettings, TabId};

/// 主机监控抽屉与全屏看板的面板实体。
pub(crate) struct MonitorView {
    /// 面板自身状态：折叠、采样档位、进程快照与筛选框。
    pub(crate) state: MonitorState,
    /// 回到外壳的弱引用；构造完成后由 `attach` 绑定。
    app: Option<WeakEntity<AppShell>>,
}

impl MonitorView {
    /// 创建面板实体；筛选输入框等子实体在这里一并建立。
    pub(crate) fn new(
        window: &mut Window,
        settings: &MonitorSettings,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            state: MonitorState::new(window, settings, cx),
            app: None,
        }
    }

    /// 绑定外壳弱引用；必须在外壳构造完成后立即调用一次。
    pub(crate) fn attach(&mut self, app: WeakEntity<AppShell>) {
        self.app = Some(app);
    }

    /// 面板是否展开。
    pub(crate) fn is_open(&self) -> bool {
        self.state.open
    }

    /// 当前抽屉宽度（像素）。
    pub(crate) fn drawer_width(&self) -> f32 {
        self.state.drawer_width
    }

    /// 写回抽屉宽度：拖拽调整与设置页共用这一入口。
    pub(crate) fn set_drawer_width(&mut self, width: f32) {
        self.state.drawer_width = width;
    }

    /// 采样档位；设置页读取当前值。
    pub(crate) fn interval_mode(&self) -> super::types::SampleIntervalMode {
        self.state.interval_mode
    }

    /// 点击表头列切换排序：换列从降序开始，同列翻转方向。
    ///
    /// 排序只在本地重排已采样的进程快照，不触发重新采样。
    pub(crate) fn toggle_process_sort(
        &mut self,
        sort: transom_core::ProcessSort,
        cx: &mut Context<Self>,
    ) {
        if self.state.sort == sort {
            self.state.sort_ascending = !self.state.sort_ascending;
        } else {
            self.state.sort = sort;
            self.state.sort_ascending = false;
        }
        cx.notify();
    }

    /// 打开或关闭抽屉；关闭时清空进程快照，避免下次打开闪一帧旧数据。
    pub(crate) fn set_open(&mut self, open: bool) {
        self.state.open = open;
        if !open {
            self.state.processes.clear();
            self.state.process_total = 0;
        }
    }

    /// 切换采样档位，返回是否真的发生变化（用于决定是否重启轮询任务）。
    pub(crate) fn set_interval_mode(&mut self, mode: super::types::SampleIntervalMode) -> bool {
        if self.state.interval_mode == mode {
            return false;
        }
        self.state.interval_mode = mode;
        true
    }

    /// 记录一轮统计样本。
    pub(crate) fn record_stats(&mut self, tab_id: TabId, stats: &transom_core::SystemStats) {
        self.state.record_stats(tab_id, stats);
    }

    /// 进程明细采样落地；总数同步取明细条数。
    pub(crate) fn set_processes(&mut self, processes: Vec<transom_core::ProcessInfo>) {
        self.state.process_total = u32::try_from(processes.len()).unwrap_or(u32::MAX);
        self.state.processes = processes;
        self.state.error = None;
    }

    /// 折叠态只回填总数。
    pub(crate) fn set_process_total(&mut self, count: u32) {
        self.state.process_total = count;
        self.state.error = None;
    }

    /// 写入面板错误提示；`None` 表示清除。
    pub(crate) fn set_error(&mut self, error: Option<String>) {
        self.state.error = error;
    }

    /// 切换某个板块的折叠态，返回进程卡片是否由折叠变为展开。
    pub(crate) fn toggle_collapse(&mut self, section: &str) -> bool {
        let collapse = &mut self.state.collapse;
        match section {
            "cpu" => collapse.cpu = !collapse.cpu,
            "memory" => collapse.memory = !collapse.memory,
            "disk" => collapse.disk = !collapse.disk,
            "network" => collapse.network = !collapse.network,
            "ports" => collapse.ports = !collapse.ports,
            "services" => collapse.services = !collapse.services,
            "processes" => {
                collapse.processes = !collapse.processes;
                if !collapse.processes {
                    return true;
                }
            }
            _ => {}
        }
        false
    }

    /// 进程卡片是否折叠（决定采样走总数还是明细）。
    pub(crate) fn processes_collapsed(&self) -> bool {
        self.state.collapse.processes
    }

    /// 各板块折叠态快照，供设置页同步展开开关。
    pub(crate) fn collapse_snapshot(&self) -> super::types::MonitorSectionCollapse {
        self.state.collapse
    }

    /// 在监视目标切换时清空走势与错误：曲线不能跨主机残留。
    pub(crate) fn reset_history(&mut self) {
        self.state.history_tab = None;
        self.state.error = None;
    }

    /// 回到外壳执行一次跨域动作。
    ///
    /// 返回 `None` 表示外壳已释放（窗口关闭中），调用方直接跳过即可。
    pub(super) fn with_shell<R>(
        &self,
        cx: &mut Context<Self>,
        action: impl FnOnce(&mut AppShell, &mut Context<AppShell>) -> R,
    ) -> Option<R> {
        let app = self.app.as_ref()?.upgrade()?;
        Some(app.update(cx, action))
    }

    /// 读取活动标签的统计快照；没有活动标签或外壳未绑定时为空。
    pub(super) fn active_stats(&self, cx: &Context<Self>) -> Option<transom_core::SystemStats> {
        let app = self.app.as_ref()?.upgrade()?;
        let shell = app.read(cx);
        shell
            .state
            .workspace
            .active_tab
            .and_then(|tab_id| shell.state.workspace.tab(tab_id))
            .and_then(|tab| tab.system_stats().cloned())
    }
}

impl Render for MonitorView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::from(cx);
        self.render_monitor_panel(window, cx, &p)
    }
}
