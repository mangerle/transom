//! 独立主机监控标签及其目标会话快照。

use super::{TabContent, WorkspaceTab};
use transom_core::{ConnectionId, Session, SessionId, TabId};

/// 监控标签关联一个真实会话，但自身不参与连接、SFTP 或终端事件。
///
/// 设计原理（Why）：监控需要展示目标会话的名称、外观和采样连接，因此保留目标
/// 会话快照与连接 ID；这些字段只通过监控专用访问器读取，不会混入普通会话标签
/// 的 `connection_id()` 或 `session_tab()` 语义。
pub(crate) struct MonitorTab {
    pub(crate) target_tab_id: TabId,
    pub(crate) target_session_id: SessionId,
    pub(crate) target_session: Session,
    pub(crate) target_connection_id: Option<ConnectionId>,
}

impl WorkspaceTab {
    /// 构造独立监控标签。
    pub(crate) fn new_monitor(
        id: TabId,
        title: String,
        target_tab_id: TabId,
        target_session: Session,
        target_connection_id: Option<ConnectionId>,
    ) -> Self {
        Self {
            id,
            title,
            status: super::TabStatus::Connected,
            content: TabContent::Monitor(Box::new(MonitorTab {
                target_tab_id,
                target_session_id: target_session.id,
                target_session,
                target_connection_id,
            })),
        }
    }

    /// 获取监控标签的专用状态。
    pub(crate) fn monitor(&self) -> Option<&MonitorTab> {
        match &self.content {
            TabContent::Monitor(monitor) => Some(monitor.as_ref()),
            _ => None,
        }
    }

    /// 获取监控目标连接；非监控标签返回 `None`。
    pub(crate) fn monitor_target_connection_id(&self) -> Option<ConnectionId> {
        self.monitor()
            .and_then(|monitor| monitor.target_connection_id)
    }
}
