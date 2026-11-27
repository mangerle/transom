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
use super::container_ops::ContainerWorkspace;
use super::lifecycle::ConnectingTab;
use super::samplers::LocalHostTabs;
use super::session_context::ManagedSessionContext;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use transom_core::{
    AppEvent, ConnectionId, KnownHostsStore, SecretStore, SessionId, TabId, TransferId,
};
use transom_git::{GitConfig, Repo};
use transom_net::SshRemote;
use transom_store::AiStore;

use super::ai_ops::AiConfigService;
use transom_sync::CloudSyncService;

/// AI 后台静默 SSH 连接的默认空闲 TTL。
pub(super) const HEADLESS_REMOTE_TTL: Duration = Duration::from_secs(30 * 60);

/// AI 后台静默 SSH 连接的最大缓存数量。
pub(super) const MAX_HEADLESS_REMOTES: usize = 8;

/// AI 工具在后台静默建立的无标签页远端 SSH 连接 LRU/TTL 缓存。
pub(super) type AiHeadlessRemoteMap = Arc<AiHeadlessRemoteCache>;

/// 有界、带空闲 TTL 的后台远端连接缓存。
pub(super) struct AiHeadlessRemoteCache {
    entries: parking_lot::Mutex<HashMap<SessionId, HeadlessRemoteEntry>>,
    ttl: Duration,
    max_entries: usize,
}

struct HeadlessRemoteEntry {
    remote: SshRemote,
    last_used: Instant,
}

impl Default for AiHeadlessRemoteCache {
    fn default() -> Self {
        Self::new(HEADLESS_REMOTE_TTL, MAX_HEADLESS_REMOTES)
    }
}

impl AiHeadlessRemoteCache {
    /// 创建指定 TTL 与容量上限的连接缓存。
    pub(super) fn new(ttl: Duration, max_entries: usize) -> Self {
        Self {
            entries: parking_lot::Mutex::new(HashMap::with_capacity(max_entries.max(1))),
            ttl,
            max_entries: max_entries.max(1),
        }
    }

    /// 取得仍有效的连接并刷新其 LRU/TTL 位置。
    pub(super) fn get(&self, session_id: &SessionId) -> Option<SshRemote> {
        let now = Instant::now();
        let mut entries = self.entries.lock();
        entries.retain(|_, entry| !self.expired(entry, now));
        let entry = entries.get_mut(session_id)?;
        if self.expired(entry, now) {
            entries.remove(session_id);
            return None;
        }
        entry.last_used = now;
        Some(entry.remote.clone())
    }

    /// 插入连接；超容量时淘汰最久未使用的连接。
    pub(super) fn insert(&self, session_id: SessionId, remote: SshRemote) {
        let now = Instant::now();
        let mut entries = self.entries.lock();
        entries.retain(|_, entry| !self.expired(entry, now));
        if !entries.contains_key(&session_id)
            && entries.len() >= self.max_entries
            && let Some(oldest) = entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(id, _)| *id)
        {
            entries.remove(&oldest);
        }
        entries.insert(
            session_id,
            HeadlessRemoteEntry {
                remote,
                last_used: now,
            },
        );
    }

    /// 清空全部后台连接；连接句柄随 `SshRemote` 释放触发底层资源回收。
    pub(super) fn clear(&self) {
        self.entries.lock().clear();
    }

    fn expired(&self, entry: &HeadlessRemoteEntry, now: Instant) -> bool {
        now.checked_duration_since(entry.last_used)
            .is_some_and(|age| age >= self.ttl)
    }
}

/// 在途 AI 运行的中途引导（Steer）注入队列映射表。
pub(super) type SteerQueueMap = Arc<
    parking_lot::Mutex<
        HashMap<String, Arc<parking_lot::Mutex<std::collections::VecDeque<String>>>>,
    >,
>;

/// 终端手动接管恢复回调映射表。
pub(super) type HandoffSenderMap = Arc<
    parking_lot::Mutex<HashMap<String, tokio::sync::oneshot::Sender<Result<Option<String>, ()>>>>,
>;

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
    /// 本地终端的系统指标与网络流量共享同一份机器级采样。
    pub(super) local_host_tabs: LocalHostTabs,
    pub(super) local_host_task: Option<tokio::task::JoinHandle<()>>,
    /// 系统统计采样开关：由 UI 按“是否有可见消费方”下发，默认开启。
    pub(super) stats_sampling: Arc<std::sync::atomic::AtomicBool>,
    /// 主机动作（拨测/唤醒）批次任务：停机时统一等待。
    pub(super) host_action_tasks: JoinSet<()>,
    /// 进程面板的一次性采样/终止任务：与主机动作同样纳入 JoinSet，
    /// 停机时统一等待，避免远端 exec 还在途时进程直接退出。
    pub(super) monitor_set: JoinSet<()>,
    /// 打开的容器管理工作区，按标签索引。
    pub(super) container_workspaces: HashMap<TabId, ContainerWorkspace>,
    /// 正在探测的引擎连接任务：同标签重连会顶掉旧任务。
    pub(super) container_connects: HashMap<TabId, tokio::task::JoinHandle<()>>,
    /// 容器一次性查询（列表/检视/启停/卷/网络）：短任务集合，停机统一等待。
    pub(super) container_ops: JoinSet<()>,
    /// Git 面板的一次性读写请求：全部子进程自带超时，停机统一限时等待。
    pub(super) git_set: JoinSet<()>,
    /// 在途 AI 运行的取消令牌。
    pub(super) ai_runs: Arc<parking_lot::Mutex<HashMap<String, CancellationToken>>>,
    /// 在途 AI 运行的中途引导（Steer）注入队列。
    pub(super) ai_steers: SteerQueueMap,
    /// 待用户审批的 AI 工具回调表。
    pub(super) ai_approvals: Arc<
        parking_lot::Mutex<
            HashMap<String, tokio::sync::oneshot::Sender<transom_core::AiApprovalDecision>>,
        >,
    >,
    /// 待用户在交互式终端完成手动操作的恢复通道。
    pub(super) ai_handoffs: HandoffSenderMap,
    /// 已通过审批卡片“允许同类”放行的命令前缀，按对话、主机与保护级别隔离。
    pub(super) ai_approval_grants: Arc<parking_lot::RwLock<transom_ai::ApprovalGrantStore>>,
    /// 待 UI 回传的终端上下文请求表。
    pub(super) ai_terminal_reqs:
        Arc<parking_lot::Mutex<HashMap<String, tokio::sync::oneshot::Sender<Option<String>>>>>,
    /// 待 UI 回传的软件级动作执行结果表。
    pub(super) ai_app_action_reqs: super::ai_ops::app_bridge::AppActionReqMap,
    /// AI 运行后台任务集合。
    pub(super) ai_set: JoinSet<()>,
    /// 托管后台任务中心（本机/远端命令、传输与子 Agent）。
    pub(super) ai_tasks: Arc<super::ai_ops::task_runner::ManagedTaskRegistry>,
    /// 子 Agent 上下文历史缓存（`subagent_id -> SessionEntry` 分支），支持 `resume_id` 多轮追问复用。
    pub(super) ai_subagent_histories:
        Arc<parking_lot::Mutex<HashMap<String, Vec<transom_ai::SessionEntry>>>>,
    /// 按对话 ID 复用的全局有界超大工具输出句柄池。
    pub(super) ai_output_stores: transom_ai::ToolOutputStorePool,
    /// AI 工具后台静默直连的远端 SSH 句柄缓存（不打开 UI 标签页）。
    pub(super) ai_headless_remotes: AiHeadlessRemoteMap,
    /// 已发起 SSH 连接的标签元数据（供反向 MCP Server 列表展示）。
    pub(super) mcp_session_meta: HashMap<TabId, transom_core::AiHostTargetDto>,
    /// 反向 MCP Server 取消令牌与后台任务句柄。
    pub(super) mcp_cancel: CancellationToken,
    pub(super) mcp_task: Option<tokio::task::JoinHandle<()>>,
}

impl ControllerState {
    /// 空状态：全部表为空，收尾与批次集合就位。
    pub(super) fn new(event_tx: flume::Sender<AppEvent>) -> Self {
        Self {
            sessions: HashMap::new(),
            sftp_tasks: HashMap::new(),
            sync_task: None,
            cleanup_set: JoinSet::new(),
            connecting: HashMap::new(),
            local_host_tabs: Arc::new(parking_lot::Mutex::new(Vec::new())),
            local_host_task: None,
            stats_sampling: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            host_action_tasks: JoinSet::new(),
            monitor_set: JoinSet::new(),
            container_workspaces: HashMap::new(),
            container_connects: HashMap::new(),
            container_ops: JoinSet::new(),
            git_set: JoinSet::new(),
            ai_runs: Arc::new(parking_lot::Mutex::new(HashMap::new())),
            ai_steers: Arc::new(parking_lot::Mutex::new(HashMap::new())),
            ai_approvals: Arc::new(parking_lot::Mutex::new(HashMap::new())),
            ai_handoffs: Arc::new(parking_lot::Mutex::new(HashMap::new())),
            ai_approval_grants: Arc::new(parking_lot::RwLock::new(
                transom_ai::ApprovalGrantStore::new(),
            )),
            ai_terminal_reqs: Arc::new(parking_lot::Mutex::new(HashMap::new())),
            ai_app_action_reqs: Arc::new(parking_lot::Mutex::new(HashMap::new())),
            ai_set: JoinSet::new(),
            ai_tasks: Arc::new(super::ai_ops::task_runner::ManagedTaskRegistry::new(
                event_tx,
            )),
            ai_subagent_histories: Arc::new(parking_lot::Mutex::new(HashMap::new())),
            ai_output_stores: transom_ai::ToolOutputStorePool::default(),
            ai_headless_remotes: Arc::new(AiHeadlessRemoteCache::default()),
            mcp_session_meta: HashMap::new(),
            mcp_cancel: CancellationToken::new(),
            mcp_task: None,
        }
    }

    /// 查询指定标签当前生效的连接 ID。
    pub(super) fn current_connection(&self, tab_id: TabId) -> Option<ConnectionId> {
        self.sessions.get(&tab_id).map(|s| s.connection_id)
    }

    /// 按连接 ID 查找仍活跃的会话及其标签。
    ///
    /// 连接 ID 全局唯一，SFTP/转发的迟到完成事件需要用它反查归属标签；
    /// 这里集中一条查找口径，避免各处各写一遍 `iter().find(...)`。
    pub(super) fn session_by_connection(
        &self,
        connection_id: ConnectionId,
    ) -> Option<(TabId, &ManagedSessionContext)> {
        find_session_by_connection(&self.sessions, connection_id)
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
        self.mcp_session_meta.remove(&tab_id);
        if let Some(session) = self.sessions.remove(&tab_id) {
            session.shutdown(&mut self.cleanup_set, reason);
        }
    }
}

/// 在会话表中按连接 ID 查找归属标签。
///
/// 以自由函数提供，供只持有 `sessions` 表的调度函数复用；连接 ID 全局唯一，
/// 同一连接不会同时归属多个标签。
pub(super) fn find_session_by_connection(
    sessions: &HashMap<TabId, ManagedSessionContext>,
    connection_id: ConnectionId,
) -> Option<(TabId, &ManagedSessionContext)> {
    sessions
        .iter()
        .find(|(_, session)| session.connection_id == connection_id)
        .map(|(tab_id, session)| (*tab_id, session))
}

/// 控制平面随控制器一次性建立的共享句柄。
pub(super) struct ControllerCtx {
    pub(super) known_hosts: Arc<dyn KnownHostsStore>,
    pub(super) secret_store: Arc<dyn SecretStore>,
    pub(super) session_store: Arc<dyn transom_core::SessionStore>,
    pub(super) settings_store: Arc<dyn transom_core::SettingsStore>,
    pub(super) backup: Arc<dyn transom_core::BackupService>,
    pub(super) git_projects: Arc<dyn transom_core::GitProjectStore>,
    pub(super) api_projects: Arc<dyn transom_api::ApiProjectStore>,
    pub(super) api_http: transom_api::ApiHttpService,
    pub(super) ai_store: Arc<AiStore>,
    pub(super) ai_config: Arc<AiConfigService>,
    pub(super) event_tx: flume::Sender<AppEvent>,
    pub(super) internal_tx: flume::Sender<Internal>,
    pub(super) cloud_sync: Arc<CloudSyncService>,
    /// 容器引擎接入服务：发现、探测与取消标志表。
    pub(super) container: Arc<transom_container::ContainerService>,
    /// 主机动作的跨批次并发闸门：用户连点两次也不会翻倍占用在途连接数。
    pub(super) action_permits: Arc<tokio::sync::Semaphore>,
    /// Git 请求登记表：界面关闭视图后按 id 丢弃迟到的子进程结果。
    pub(super) git: Arc<transom_git::GitService>,
    /// Git 子进程配置的唯一快照：界面只投递 `GitCommand::Configure`，
    /// 控制平面写入，任务派发时读取一次并绑定到 [`Repo`]。
    pub(super) git_config: Arc<RwLock<GitConfig>>,
    /// 本地进程采样器：跨请求保留上一轮进程表，CPU 使用率才能形成差值。
    pub(super) local_processes: Arc<parking_lot::Mutex<transom_platform::system::ProcessSampler>>,
}

impl ControllerCtx {
    /// 取当前 Git 配置的不可变快照。
    ///
    /// 设计原理（Why）：返回拥有型 [`Arc`] 而不是守卫，任务可以跨 `await`
    /// 安全持有快照，配置写入也不会阻塞或被子进程执行时长拖住。
    pub(super) fn git_config_snapshot(&self) -> Arc<GitConfig> {
        Arc::new(self.git_config.read().clone())
    }

    /// 以当前配置快照构造仓库句柄。
    ///
    /// 所有 Git 命令派发统一走这里：分支不得各自读取配置或使用默认配置的
    /// [`Repo::at`]，否则用户设置的可执行文件与作者会在该分支悄悄失效。
    pub(super) fn git_repo(&self, path: PathBuf) -> Repo {
        Repo::with_config(path, self.git_config_snapshot())
    }
}

#[cfg(test)]
mod tests {
    use super::find_session_by_connection;
    use crate::controller::session_context::ManagedSessionContext;
    use std::collections::HashMap;
    use transom_core::{ConnectionId, TabId};

    /// 按连接 ID 反查归属标签；过期连接 ID 必须查不到。
    #[test]
    fn finds_session_by_connection_and_rejects_stale_ids() {
        let mut sessions = HashMap::new();
        let tab = TabId::new();
        let connection = ConnectionId::new();
        sessions.insert(tab, ManagedSessionContext::new(connection, None));

        let (found_tab, _) =
            find_session_by_connection(&sessions, connection).expect("活跃连接必须能反查标签");
        assert_eq!(found_tab, tab);
        assert!(
            find_session_by_connection(&sessions, ConnectionId::new()).is_none(),
            "过期连接 ID 不得命中任何会话"
        );
    }
}
