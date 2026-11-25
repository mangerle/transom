//! 端口转发事件。

use crate::error::AppError;
use crate::ids::{ConnectionId, ForwardId, TabId};
use crate::model::forward::ForwardStatus;

/// 端口转发事件。
pub enum ForwardAppEvent {
    ForwardChanged {
        tab_id: TabId,
        connection_id: ConnectionId,
        status: ForwardStatus,
    },
    /// 目标端口连通性探测结果：一次性回报，界面显示在对应规则卡片下。
    ForwardProbeResult {
        tab_id: TabId,
        connection_id: ConnectionId,
        rule_id: ForwardId,
        result: Result<(), AppError>,
    },
}
