//! 容器端口建立 SSH 本地端口转发弹窗数据模型。
//!
//! 设计原理（Why）：将远程容器的内部服务端口通过当前活跃的 SSH 隧道直接映射到本地，
//! 开发者无需配置复杂的隧道命令，即可在本地电脑无缝调试容器内 Redis、MySQL 或 HTTP 服务。

use gpui_kit::Entity;
use gpui_kit::component::input::InputState;
use transom_core::{ConnectionId, TabId};

/// 容器端口建立本地端口转发弹窗状态。
pub(crate) struct ContainerForwardDialog {
    pub(crate) container_name: String,
    pub(crate) remote_port: u16,
    pub(crate) local_port_input: Entity<InputState>,
    pub(crate) target_host_input: Entity<InputState>,
    /// 可用 SSH 会话列表：(会话名称, tab_id, connection_id)
    pub(crate) ssh_sessions: Vec<(String, TabId, ConnectionId)>,
    pub(crate) selected_ssh_index: usize,
}
