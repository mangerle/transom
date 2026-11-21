//! 终端会话标签及其窗格运行状态。

use crate::sftp::SftpPanelView;
use crate::zmodem::ZmodemState;
use gpui_kit::{Entity, Subscription, Task};
use std::collections::HashMap;
use transom_core::{
    ConnectionId, ForwardId, ForwardStatus, NetworkTraffic, Session, SessionId, SystemStats,
};
use transom_terminal::{ProgressState, TerminalView};

/// OSC 9;4 上报的标签任务进度快照。
///
/// 只保留最近一次上报：进度展示是即时状态而非历史，`Remove` 事件直接把
/// 整个字段清回 `None`，不保留残留条。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TabProgress {
    pub(crate) state: ProgressState,
    pub(crate) progress: u8,
}

/// 实时通信会话标签的全部专属状态。
///
/// 设计原理（Why）：`Session`、连接 ID、OSC 7 工作目录和终端窗格任务只在
/// 真实连接标签中存在；集中到本结构后，非会话标签无法再表示这些字段。
pub(crate) struct SessionTab {
    pub(crate) session_id: SessionId,
    pub(crate) session: Session,
    pub(crate) connection_id: Option<ConnectionId>,
    /// 终端最近一次通过 OSC 7 上报的工作目录。
    pub(crate) cwd: Option<String>,
    /// 终端最近一次通过 OSC 9;4 上报的任务进度；`None` 表示无活动进度。
    pub(crate) progress: Option<TabProgress>,
    pub(crate) pane: SessionPaneState,
}

/// 实时通信会话窗格关联的所有运行期状态。
#[derive(Default)]
pub(crate) struct SessionPaneState {
    pub(crate) terminal: Option<Entity<TerminalView>>,
    /// 该窗格 SFTP 面板的实体句柄；实体随窗格强持有，置 `None` 即销毁。
    pub(crate) sftp_view: Option<Entity<SftpPanelView>>,
    pub(crate) forwards_view: Option<Entity<crate::forward::ForwardView>>,
    pub(crate) forward_statuses: HashMap<ForwardId, ForwardStatus>,
    /// 目标端口连通性探测结果（预本地化文案；`None` 表示成功），一次性展示。
    pub(crate) forward_probe_results: HashMap<ForwardId, Option<String>>,
    pub(crate) system_stats: Option<SystemStats>,
    pub(crate) network_traffic: Option<NetworkTraffic>,
    pub(crate) _startup_task: Option<Task<()>>,
    pub(crate) _subscription: Option<Subscription>,
    pub(crate) reconnect_attempts: u32,
    pub(crate) reconnect_task: Option<Task<()>>,
    pub(crate) zmodem: Option<ZmodemState>,
}
