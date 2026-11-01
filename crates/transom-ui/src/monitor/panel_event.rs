//! 监控面板发给外壳的意图事件。
//!
//! 面板不持有外壳句柄：折叠/档位/宽度只改自身状态，需要外壳收尾（采样、重启轮询、
//! 调整分栏、脱离为标签页）时发事件，外壳在创建面板时订阅一次。
//! 事件自带全部数据，外壳处理时不得回读面板。

use std::time::Duration;
use transom_core::TabId;

/// 监控面板向外壳发出的意图。
#[derive(Clone)]
pub(crate) enum MonitorPanelEvent {
    /// 板块折叠状态变化；外壳据此落盘展开开关，必要时补一轮采样。
    CollapseChanged {
        collapse: crate::monitor::MonitorSectionCollapse,
        expanded_processes: bool,
    },
    /// 采样档位变化，外壳据此重启轮询任务。
    RestartSampling(Duration),
    /// 请求立即采样一轮进程。
    SampleProcesses { collapsed: bool },
    /// 抽屉宽度已改，外壳据此持久化并调整分栏。
    DrawerWidthChanged(f32),
    /// 请求把抽屉脱离为独立标签页。
    MaximizeToTab,
    /// 请求把独立标签页收回抽屉。
    DockToDrawer { tab_id: TabId, target_tab_id: TabId },
    /// 请在终端里追踪进程。
    TraceProcess(u32),
    /// 请关闭指定窗格（监控看板里的"关闭标签"）。
    ClosePane { tab_id: TabId },
    /// 请在终端里重启 systemd 服务。
    RestartService(String),
    /// 请终止进程；`pid <= 1` 的拦截已在面板内完成。
    KillProcess { pid: u32, force: bool },
}
