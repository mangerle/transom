//! AI 控制平面的运行态、审批通道、后台任务与反向 MCP 生命周期。
//!
//! 设计原理（Why）：这些字段都随 AI 子系统整体启停、清理和回收，与 SSH、
//! SFTP、容器等会话资源生命周期不同。集中到 [`AiControlState`] 后，控制器只
//! 暴露一个 AI 状态入口，停机顺序与跨表清理也由 AI 领域自己维护。

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::{Mutex, RwLock};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use transom_ai::{ApprovalGrantStore, ApprovalRequestId, RunId, SessionEntry, ToolOutputStorePool};
use transom_core::{AiApprovalDecision, AiHostTargetDto, AppEvent, SessionId, TabId};
use transom_net::SshRemote;

use super::app_bridge::AppActionReqMap;
use super::task_runner::ManagedTaskRegistry;

/// AI 后台静默 SSH 连接的默认空闲 TTL。
pub(in crate::controller) const HEADLESS_REMOTE_TTL: Duration = Duration::from_secs(30 * 60);

/// AI 后台静默 SSH 连接的最大缓存数量。
pub(in crate::controller) const MAX_HEADLESS_REMOTES: usize = 8;

/// AI 工具在后台静默建立的无标签页远端 SSH 连接 LRU/TTL 缓存。
pub(in crate::controller) type AiHeadlessRemoteMap = Arc<AiHeadlessRemoteCache>;

/// 在途 AI 运行取消令牌映射。
pub(in crate::controller) type AiRunMap = Arc<Mutex<HashMap<RunId, CancellationToken>>>;

/// 待审批请求回传通道映射。
pub(in crate::controller) type AiApprovalMap =
    Arc<Mutex<HashMap<ApprovalRequestId, tokio::sync::oneshot::Sender<AiApprovalDecision>>>>;

/// 在途 AI 运行的中途引导（Steer）注入队列映射表。
pub(in crate::controller) type SteerQueueMap =
    Arc<Mutex<HashMap<RunId, Arc<Mutex<VecDeque<String>>>>>>;

/// 终端手动接管恢复回调映射表。
pub(in crate::controller) type HandoffSenderMap =
    Arc<Mutex<HashMap<String, tokio::sync::oneshot::Sender<Result<Option<String>, ()>>>>>;

/// 有界、带空闲 TTL 的后台远端连接缓存。
pub(in crate::controller) struct AiHeadlessRemoteCache {
    entries: Mutex<HashMap<SessionId, HeadlessRemoteEntry>>,
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
    pub(in crate::controller) fn new(ttl: Duration, max_entries: usize) -> Self {
        Self {
            entries: Mutex::new(HashMap::with_capacity(max_entries.max(1))),
            ttl,
            max_entries: max_entries.max(1),
        }
    }

    /// 取得仍有效的连接并刷新其 LRU/TTL 位置。
    pub(in crate::controller) fn get(&self, session_id: &SessionId) -> Option<SshRemote> {
        let now = Instant::now();
        let mut entries = self.entries.lock();
        entries.retain(|_, entry| !self.expired(entry, now));
        let entry = entries.get_mut(session_id)?;
        entry.last_used = now;
        Some(entry.remote.clone())
    }

    /// 插入连接；超容量时淘汰最久未使用的连接。
    pub(in crate::controller) fn insert(&self, session_id: SessionId, remote: SshRemote) {
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
    pub(in crate::controller) fn clear(&self) {
        self.entries.lock().clear();
    }

    fn expired(&self, entry: &HeadlessRemoteEntry, now: Instant) -> bool {
        now.checked_duration_since(entry.last_used)
            .is_some_and(|age| age >= self.ttl)
    }
}

/// AI 子系统的全部控制态。
pub(in crate::controller) struct AiControlState {
    /// 在途 AI 运行的取消令牌。
    pub(in crate::controller) runs: AiRunMap,
    /// 在途 AI 运行的中途引导注入队列。
    pub(in crate::controller) steers: SteerQueueMap,
    /// 待用户审批的 AI 工具回调表。
    pub(in crate::controller) approvals: AiApprovalMap,
    /// 待用户在交互式终端完成手动操作的恢复通道。
    pub(in crate::controller) handoffs: HandoffSenderMap,
    /// 已通过审批卡片“允许同类”放行的命令前缀。
    pub(in crate::controller) approval_grants: Arc<RwLock<ApprovalGrantStore>>,
    /// 待 UI 回传的终端上下文请求表。
    pub(in crate::controller) terminal_reqs:
        Arc<Mutex<HashMap<String, tokio::sync::oneshot::Sender<Option<String>>>>>,
    /// 待 UI 回传的软件级动作执行结果表。
    pub(in crate::controller) app_action_reqs: AppActionReqMap,
    /// AI 运行后台任务集合。
    pub(in crate::controller) run_set: JoinSet<()>,
    /// 托管后台任务中心（本机/远端命令、传输与子 Agent）。
    pub(in crate::controller) managed_tasks: Arc<ManagedTaskRegistry>,
    /// 子 Agent 上下文历史缓存，支持 `resume_id` 多轮追问复用。
    pub(in crate::controller) subagent_histories: Arc<Mutex<HashMap<String, Vec<SessionEntry>>>>,
    /// 按对话 ID 复用的全局有界超大工具输出句柄池。
    pub(in crate::controller) output_stores: ToolOutputStorePool,
    /// AI 工具后台静默直连的远端 SSH 句柄缓存。
    pub(in crate::controller) headless_remotes: AiHeadlessRemoteMap,
    /// 已发起 SSH 连接的标签元数据（供反向 MCP Server 列表展示）。
    pub(in crate::controller) mcp_session_meta: HashMap<TabId, AiHostTargetDto>,
    /// 反向 MCP Server 取消令牌与后台任务句柄。
    pub(in crate::controller) mcp_cancel: CancellationToken,
    pub(in crate::controller) mcp_task: Option<tokio::task::JoinHandle<()>>,
}

impl AiControlState {
    /// 创建空 AI 控制态。
    pub(in crate::controller) fn new(event_tx: flume::Sender<AppEvent>) -> Self {
        Self {
            runs: Arc::new(Mutex::new(HashMap::new())),
            steers: Arc::new(Mutex::new(HashMap::new())),
            approvals: Arc::new(Mutex::new(HashMap::new())),
            handoffs: Arc::new(Mutex::new(HashMap::new())),
            approval_grants: Arc::new(RwLock::new(ApprovalGrantStore::new())),
            terminal_reqs: Arc::new(Mutex::new(HashMap::new())),
            app_action_reqs: Arc::new(Mutex::new(HashMap::new())),
            run_set: JoinSet::new(),
            managed_tasks: Arc::new(ManagedTaskRegistry::new(event_tx)),
            subagent_histories: Arc::new(Mutex::new(HashMap::new())),
            output_stores: ToolOutputStorePool::default(),
            headless_remotes: Arc::new(AiHeadlessRemoteCache::default()),
            mcp_session_meta: HashMap::new(),
            mcp_cancel: CancellationToken::new(),
            mcp_task: None,
        }
    }

    /// 取消所有 AI 与反向 MCP 工作，并限时等待后台任务回收。
    pub(in crate::controller) async fn shutdown(&mut self) {
        self.mcp_cancel.cancel();
        self.managed_tasks.cancel_all();
        if let Some(task) = self.mcp_task.take() {
            let _ = tokio::time::timeout(Duration::from_secs(1), task).await;
        }
        {
            let runs = self.runs.lock();
            for token in runs.values() {
                token.cancel();
            }
        }
        self.approvals.lock().clear();
        self.handoffs.lock().clear();
        self.terminal_reqs.lock().clear();
        self.app_action_reqs.lock().clear();
        self.output_stores.clear();
        self.approval_grants.write().clear();
        self.headless_remotes.clear();
        let _ = tokio::time::timeout(Duration::from_secs(2), async {
            while self.run_set.join_next().await.is_some() {}
        })
        .await;
        self.managed_tasks.close_background();
        if tokio::time::timeout(Duration::from_secs(2), self.managed_tasks.wait_background())
            .await
            .is_err()
        {
            log::warn!("等待 AI 托管后台任务退出超过 2 秒");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::AiHeadlessRemoteCache;

    #[test]
    fn headless_cache_clamps_zero_capacity_to_one() {
        let cache = AiHeadlessRemoteCache::new(Duration::from_secs(1), 0);
        assert_eq!(cache.max_entries, 1);
    }
}
