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
    /// 容器内服务端口。
    pub(crate) private_port: u16,
    /// 已发布到宿主机的端口；未发布时转发目标只能退回容器端口。
    pub(crate) published_port: Option<u16>,
    pub(crate) local_port_input: Entity<InputState>,
    pub(crate) target_host_input: Entity<InputState>,
    /// 可用 SSH 会话列表：(会话名称, tab_id, connection_id)
    pub(crate) ssh_sessions: Vec<(String, TabId, ConnectionId)>,
    pub(crate) selected_ssh_index: usize,
}

/// 打开端口转发弹窗所需的参数集合。
///
/// 设计原理（Why）：打开动作同时需要容器身份、端口信息、两个输入控件与
/// 候选 SSH 会话；平铺参数会超过 clippy 的参数上限，且调用点难以看清字段归属。
pub(crate) struct ContainerForwardOpenParams {
    /// 目标容器名（已去掉 Docker 的 `/` 前缀）。
    pub(crate) container_name: String,
    /// 容器内私有端口。
    pub(crate) private_port: u16,
    /// 已发布到宿主机的端口。
    pub(crate) published_port: Option<u16>,
    /// 本地监听端口输入框。
    pub(crate) local_port_input: Entity<InputState>,
    /// 目标主机输入框。
    pub(crate) target_host_input: Entity<InputState>,
    /// 可用 SSH 会话列表：(会话名称, tab_id, connection_id)。
    pub(crate) ssh_sessions: Vec<(String, TabId, ConnectionId)>,
}
