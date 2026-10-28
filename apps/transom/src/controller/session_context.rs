//! 托管会话上下文：单一事实来源管理单个会话的全部活动资源。
//!
//! 设计原理（Why）：
//! - 实现初衷：消除原先平铺在 ControllerState 中的 9+ 张 HashMap 字典，
//!   解决连接断开时需手动从各个字典逐项删除导致的资源泄漏与幽灵任务问题；
//! - 核心优势：
//!   1. 会话绑定的所有资源（网络句柄、SFTP 缓存、端口转发、监控采样、ZMODEM）生命周期完全内聚；
//!   2. 统一提供 `shutdown()` 方法，一键安全回收全部异步任务与底层连接；
//!   3. 消除连接接入时复杂的借用表对象（`TabCleanupTables`）；
//! - 代价与局限：跨会话批量操作（如全停机）需要遍历 `sessions` 值集合。

use std::collections::HashMap;
use tokio::task::{JoinHandle, JoinSet};
use transom_core::{ConnectionId, ForwardId};
use transom_net::{ForwardTask, SftpClient, SshRemote};
use transom_zmodem::ZmodemControl;

/// 托管会话上下文。
pub(crate) struct ManagedSessionContext {
    pub(crate) connection_id: ConnectionId,
    pub(crate) remote: Option<SshRemote>,
    pub(crate) sftp_client: Option<SftpClient>,
    pub(crate) transfer_sftp_client: Option<SftpClient>,
    pub(crate) forwards: HashMap<ForwardId, ForwardTask>,
    pub(crate) traffic_task: Option<JoinHandle<()>>,
    pub(crate) key_deploy_task: Option<JoinHandle<()>>,
    pub(crate) stats_task: Option<JoinHandle<()>>,
    pub(crate) zmodem_task: Option<JoinHandle<()>>,
    pub(crate) zmodem_control: Option<ZmodemControl>,
}

impl ManagedSessionContext {
    /// 构造新的会话上下文。
    pub(crate) fn new(connection_id: ConnectionId, remote: Option<SshRemote>) -> Self {
        Self {
            connection_id,
            remote,
            sftp_client: None,
            transfer_sftp_client: None,
            forwards: HashMap::new(),
            traffic_task: None,
            key_deploy_task: None,
            stats_task: None,
            zmodem_task: None,
            zmodem_control: None,
        }
    }

    /// 停止当前会话绑定的流量采样。
    pub(super) fn stop_traffic(&mut self) {
        if let Some(task) = self.traffic_task.take() {
            task.abort();
        }
    }

    /// 停止当前会话绑定的公钥部署任务。
    pub(super) fn stop_key_deploy(&mut self) {
        if let Some(task) = self.key_deploy_task.take() {
            task.abort();
        }
    }

    /// 停止当前会话绑定的系统监控指标采样。
    pub(super) fn stop_stats(&mut self) {
        if let Some(task) = self.stats_task.take() {
            task.abort();
        }
    }

    /// 停止当前会话绑定的 ZMODEM 监督传输。
    pub(super) fn stop_zmodem(&mut self) {
        if let Some(task) = self.zmodem_task.take() {
            task.abort();
        }
        self.zmodem_control = None;
    }

    /// 停止所有在途端口转发任务。
    pub(super) fn stop_forwards(&mut self, cleanup: &mut JoinSet<()>) {
        for (_, task) in self.forwards.drain() {
            cleanup.spawn(async move {
                task.stop().await;
            });
        }
    }

    /// 销毁并回收会话持有的全部异步任务与网络连接。
    pub(super) fn shutdown(mut self, cleanup: &mut JoinSet<()>, disconnect_reason: &str) {
        self.stop_traffic();
        self.stop_key_deploy();
        self.stop_stats();
        self.stop_zmodem();
        self.stop_forwards(cleanup);
        self.sftp_client = None;
        self.transfer_sftp_client = None;

        if let Some(remote) = self.remote.take() {
            let reason = disconnect_reason.to_string();
            cleanup.spawn(async move {
                remote.disconnect(&reason).await;
            });
        }
    }

    /// 设置流量采样任务句柄，若已有旧任务则先行中止。
    pub(super) fn set_traffic_task(&mut self, task: JoinHandle<()>) {
        self.stop_traffic();
        self.traffic_task = Some(task);
    }

    /// 设置系统统计采样任务句柄，若已有旧任务则先行中止。
    pub(super) fn set_stats_task(&mut self, task: JoinHandle<()>) {
        self.stop_stats();
        self.stats_task = Some(task);
    }

    /// 设置公钥部署任务句柄，若已有旧任务则先行中止。
    pub(super) fn set_key_deploy_task(&mut self, task: JoinHandle<()>) {
        self.stop_key_deploy();
        self.key_deploy_task = Some(task);
    }
}

#[cfg(test)]
mod tests {
    use super::ManagedSessionContext;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::task::JoinSet;
    use transom_core::ConnectionId;

    fn pending_task(cancelled: Arc<AtomicUsize>) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            struct Guard(Arc<AtomicUsize>);
            impl Drop for Guard {
                fn drop(&mut self) {
                    self.0.fetch_add(1, Ordering::Relaxed);
                }
            }
            let _guard = Guard(cancelled);
            std::future::pending::<()>().await;
        })
    }

    /// 断开清理必须取消会话绑定的全部后台任务，且无远端时不产生收尾任务。
    #[tokio::test]
    async fn shutdown_aborts_session_tasks() {
        let cancelled = Arc::new(AtomicUsize::new(0));
        let mut context = ManagedSessionContext::new(ConnectionId::new(), None);
        context.set_stats_task(pending_task(cancelled.clone()));
        context.set_traffic_task(pending_task(cancelled.clone()));
        context.set_key_deploy_task(pending_task(cancelled.clone()));

        // 先让每个任务跑到 pending 并建立 Drop 守卫，再验证取消路径。
        tokio::task::yield_now().await;
        let mut cleanup = JoinSet::new();
        context.shutdown(&mut cleanup, "测试断开");
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while cancelled.load(Ordering::Relaxed) < 3 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("断开清理必须在限定时间内取消全部后台任务");
        assert!(cleanup.is_empty(), "无远端连接时不应产生断开收尾任务");
    }
}
