//! 控制平面的状态表与共享上下文。
//!
//! 设计原理（Why）：控制循环此前把十二张任务/连接表平铺在函数栈上，四个
//! 连接接入分支各自把整组表逐个重列一遍——新增一张表要改五处，接入路径
//! 长期存在四份复制。收敛为 [`ControllerState`] 后，新增状态只改本文件与
//! 相关处理函数，接入路径只剩一条。
//!
//! [`ControllerCtx`] 收纳随控制器一次性建立的共享句柄；处理函数只读它们，
//! 不再从主循环逐个透传或克隆。

use super::Internal;
use super::lifecycle::ConnectingTab;
use super::samplers::LocalTrafficTabs;
use super::session_context::ManagedSessionContext;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::task::JoinSet;
use transom_core::{AppEvent, ConnectionId, KnownHostsStore, TabId, TransferId};
use transom_net::SshRemote;
use transom_sync::CloudSyncService;

/// 控制平面的全部可变状态表。
pub(super) struct ControllerState {
    /// 统一托管的活动会话上下文表：按 TabId 索引其连接、远端、SFTP、转发与监控任务。
    pub(super) sessions: HashMap<TabId, ManagedSessionContext>,
    /// 在途 SFTP 操作任务：取消或关机时中止。
    pub(super) sftp_tasks: HashMap<(TabId, TransferId), tokio::task::JoinHandle<()>>,
    /// 云同步任务：全局同一时刻只允许一个在途，进行中时拒绝新命令。
    pub(super) sync_task: Option<tokio::task::JoinHandle<()>>,
    /// 收尾任务集合（断开远端、转发优雅停止）：集中纳管 JoinHandle，
    /// 停机时统一限时等待，任务不再脱缰。
    pub(super) cleanup_set: JoinSet<()>,
    /// 待连接意图表：发起连接即登记，完成事件必须匹配当前意图才接入；
    /// 同标签的新连接会顶掉并取消旧建连任务，迟到的旧结果被拒收。
    pub(super) connecting: HashMap<TabId, ConnectingTab>,
    /// 本地终端共享一份系统统计采集：/proc 读取与 df 子进程按机器计一次，
    /// 采样器在首个本地连接时拉起、标签清空后退出。
    pub(super) local_stats_tabs: Arc<std::sync::Mutex<Vec<(TabId, ConnectionId)>>>,
    pub(super) local_stats_task: Option<tokio::task::JoinHandle<()>>,
    /// 本地网络流量同样是机器级属性：共享采样按机器计一份读取，逐标签广播。
    pub(super) local_traffic_tabs: LocalTrafficTabs,
    pub(super) local_traffic_task: Option<tokio::task::JoinHandle<()>>,
    /// 主机动作（拨测/唤醒）批次任务：停机时统一等待。
    pub(super) host_action_tasks: JoinSet<()>,
    /// 进程面板的一次性采样/终止任务：与主机动作同样纳入 JoinSet，
    /// 停机时统一等待，避免远端 exec 还在途时进程直接退出。
    pub(super) monitor_set: JoinSet<()>,
}

impl ControllerState {
    /// 空状态：全部表为空，收尾与批次集合就位。
    pub(super) fn new() -> Self {
        Self {
            sessions: HashMap::new(),
            sftp_tasks: HashMap::new(),
            sync_task: None,
            cleanup_set: JoinSet::new(),
            connecting: HashMap::new(),
            local_stats_tabs: Arc::new(std::sync::Mutex::new(Vec::new())),
            local_stats_task: None,
            local_traffic_tabs: Arc::new(std::sync::Mutex::new(Vec::new())),
            local_traffic_task: None,
            host_action_tasks: JoinSet::new(),
            monitor_set: JoinSet::new(),
        }
    }

    /// 查询指定标签当前生效的连接 ID。
    pub(super) fn current_connection(&self, tab_id: TabId) -> Option<ConnectionId> {
        self.sessions.get(&tab_id).map(|s| s.connection_id)
    }

    /// 查询指定标签当前生效且连接 ID 匹配的 SSH 远端连接。
    pub(super) fn current_remote(
        &self,
        tab_id: TabId,
        connection_id: ConnectionId,
    ) -> Option<SshRemote> {
        self.sessions
            .get(&tab_id)
            .filter(|s| s.connection_id == connection_id)
            .and_then(|s| s.remote.clone())
    }

    /// 销毁并安全清理指定标签的全部会话资源。
    pub(super) fn terminate_session(&mut self, tab_id: TabId, reason: &str) {
        if let Some(session) = self.sessions.remove(&tab_id) {
            session.shutdown(&mut self.cleanup_set, reason);
        }
    }
}

/// 控制平面随控制器一次性建立的共享句柄。
pub(super) struct ControllerCtx {
    pub(super) known_hosts: Arc<dyn KnownHostsStore>,
    pub(super) event_tx: flume::Sender<AppEvent>,
    pub(super) internal_tx: flume::Sender<Internal>,
    pub(super) cloud_sync: Arc<CloudSyncService>,
    /// 主机动作的跨批次并发闸门：用户连点两次也不会翻倍占用在途连接数。
    pub(super) action_permits: Arc<tokio::sync::Semaphore>,
}
