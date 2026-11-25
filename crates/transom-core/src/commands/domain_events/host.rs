//! 主机监控、遥测、拨测与运维动作事件。

use crate::commands::SystemStats;
use crate::error::AppError;
use crate::ids::{ConnectionId, SessionId, TabId};
use crate::model::host::HostOutcome;
use crate::protocol::system_stats::ProcessInfo;
use crate::protocol::traffic::NetworkTraffic;

/// 主机监控、遥测、拨测与运维动作事件。
pub enum HostAppEvent {
    SystemStatsUpdated {
        tab_id: TabId,
        connection_id: ConnectionId,
        stats: Box<SystemStats>,
    },
    NetworkTrafficUpdated {
        tab_id: TabId,
        connection_id: ConnectionId,
        traffic: NetworkTraffic,
    },
    PublicKeyDeployed {
        tab_id: TabId,
        connection_id: ConnectionId,
        key_name: String,
        error: Option<AppError>,
    },
    HostActionResult {
        session_id: SessionId,
        outcome: HostOutcome,
    },
    ProcessesSampled {
        tab_id: TabId,
        connection_id: ConnectionId,
        processes: Vec<ProcessInfo>,
    },
    /// 折叠态下的进程总数采样结果。
    ProcessCountSampled {
        tab_id: TabId,
        connection_id: ConnectionId,
        count: u32,
    },
    ProcessesSampleFailed {
        tab_id: TabId,
        connection_id: ConnectionId,
        error: AppError,
    },
    ProcessKilled {
        tab_id: TabId,
        connection_id: ConnectionId,
        pid: u32,
        error: Option<AppError>,
    },
}
