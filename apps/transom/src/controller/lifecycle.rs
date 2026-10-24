//! 连接生命周期助手：停止各域后台任务与三类连接的公共接入路径。

use std::collections::HashMap;
use std::sync::Arc;
use tokio::task::JoinSet;
use transom_core::{
    AppError, AppEvent, ConnectionId, ErrorCode, ForwardId, TabId, TerminalIo, TransferId,
};
use transom_net::{ForwardTask, SftpClient, SshRemote};

use super::Internal;

/// 注销一个标签的本地统计订阅；订阅清空后采样器自行退出。
pub(super) fn drop_local_stats(
    local_stats_tabs: &Arc<std::sync::Mutex<Vec<(TabId, ConnectionId)>>>,
    tab_id: TabId,
) {
    if let Ok(mut tabs) = local_stats_tabs.lock() {
        tabs.retain(|(tab, _)| *tab != tab_id);
    }
}

pub(super) fn stop_forwards(
    forwards: &mut HashMap<TabId, HashMap<ForwardId, ForwardTask>>,
    cleanup: &mut JoinSet<()>,
    tab_id: TabId,
) {
    if let Some(tasks) = forwards.remove(&tab_id) {
        for (_, task) in tasks {
            // 转发停止要等服务端确认拆除，纳入收尾集合等待而非丢弃句柄
            cleanup.spawn(async move {
                task.stop().await;
            });
        }
    }
}

pub(super) fn stop_stats(
    stats_tasks: &mut HashMap<TabId, tokio::task::JoinHandle<()>>,
    tab_id: TabId,
) {
    if let Some(task) = stats_tasks.remove(&tab_id) {
        task.abort();
    }
}

pub(super) fn stop_traffic(
    traffic_tasks: &mut HashMap<TabId, tokio::task::JoinHandle<()>>,
    tab_id: TabId,
) {
    if let Some(task) = traffic_tasks.remove(&tab_id) {
        task.abort();
    }
}

pub(super) fn stop_key_deploy(
    key_deploy_tasks: &mut HashMap<TabId, tokio::task::JoinHandle<()>>,
    tab_id: TabId,
) {
    if let Some(task) = key_deploy_tasks.remove(&tab_id) {
        task.abort();
    }
}

/// 连接接入时需要清理的各状态表借用集合，收敛参数个数（规范：超过
/// 5 个参数必须参数对象化）。
pub(super) struct TabCleanupTables<'a> {
    pub(super) current_connections: &'a mut HashMap<TabId, ConnectionId>,
    pub(super) forwards: &'a mut HashMap<TabId, HashMap<ForwardId, ForwardTask>>,
    pub(super) traffic_tasks: &'a mut HashMap<TabId, tokio::task::JoinHandle<()>>,
    pub(super) key_deploy_tasks: &'a mut HashMap<TabId, tokio::task::JoinHandle<()>>,
    pub(super) stats_tasks: &'a mut HashMap<TabId, tokio::task::JoinHandle<()>>,
    pub(super) sftp_clients: &'a mut HashMap<TabId, SftpClient>,
    pub(super) remotes: &'a mut HashMap<TabId, (ConnectionId, SshRemote)>,
    pub(super) cleanup: &'a mut JoinSet<()>,
}

/// 新连接接入的登记参数。
///
/// `join` 是连接生命周期的终结信号：SSH 连接是任务 JoinHandle，本地/
/// 串口是 PTY 线程的 oneshot 接收端；统一擦除为 `Output = ()` 的 Future，
/// 出口监听只关心"何时结束"，不关心结束方式。
pub(super) struct AdoptConnection<J> {
    pub(super) tab_id: TabId,
    pub(super) connection_id: ConnectionId,
    pub(super) io: TerminalIo,
    pub(super) join: J,
    /// SSH 连接的新远端句柄；本地/串口连接为 None，只清理旧远端。
    pub(super) remote: Option<SshRemote>,
    /// 顶掉旧连接时的断开说明，用于区分"SSH 重连"与"切回本地/串口"。
    pub(super) disconnect_reason: &'static str,
}

/// 三类连接（SSH/本地/串口）接入的公共路径：登记当前连接、停掉旧连接的
/// 全部后台任务、断开被顶掉的旧远端、广播 TerminalReady 并挂接退出监听。
///
/// 设计原理（Why）：三个分支此前各自复制这段清理逻辑，新增连接种类或
/// 清理项时极易漏改其中一处；分支里只保留各自的差异化收尾（流量采样、
/// 系统统计）。
pub(super) async fn adopt_connection<J>(
    tables: TabCleanupTables<'_>,
    setup: AdoptConnection<J>,
    event_tx: &flume::Sender<AppEvent>,
    internal_tx: &flume::Sender<Internal>,
) where
    J: std::future::Future<Output = ()> + Send + 'static,
{
    let tab_id = setup.tab_id;
    let connection_id = setup.connection_id;
    tables.current_connections.insert(tab_id, connection_id);
    stop_forwards(tables.forwards, tables.cleanup, tab_id);
    stop_traffic(tables.traffic_tasks, tab_id);
    stop_key_deploy(tables.key_deploy_tasks, tab_id);
    stop_stats(tables.stats_tasks, tab_id);
    tables.sftp_clients.remove(&tab_id);
    // SSH 重连用 insert 顶掉旧远端；切回本地/串口则移除旧远端并断开
    let old = match setup.remote {
        Some(remote) => tables.remotes.insert(tab_id, (connection_id, remote)),
        None => tables.remotes.remove(&tab_id),
    };
    if let Some((_, old_remote)) = old {
        let reason = setup.disconnect_reason.to_string();
        tables.cleanup.spawn(async move {
            old_remote.disconnect(&reason).await;
        });
    }
    let _ = event_tx
        .send_async(AppEvent::TerminalReady {
            tab_id,
            connection_id,
            io: setup.io,
        })
        .await;
    let internal_tx = internal_tx.clone();
    tokio::spawn(async move {
        let _ = setup.join.await;
        let _ = internal_tx
            .send_async(Internal::Disconnected {
                tab_id,
                connection_id,
            })
            .await;
    });
}

/// 优雅停机的状态表借用集合。
pub(super) struct ShutdownTables<'a> {
    pub(super) forwards: &'a mut HashMap<TabId, HashMap<ForwardId, ForwardTask>>,
    pub(super) traffic_tasks: &'a mut HashMap<TabId, tokio::task::JoinHandle<()>>,
    pub(super) key_deploy_tasks: &'a mut HashMap<TabId, tokio::task::JoinHandle<()>>,
    pub(super) stats_tasks: &'a mut HashMap<TabId, tokio::task::JoinHandle<()>>,
    pub(super) sftp_transfers: &'a mut HashMap<(TabId, TransferId), tokio::task::JoinHandle<()>>,
    pub(super) remotes: &'a mut HashMap<TabId, (ConnectionId, SshRemote)>,
    pub(super) cleanup: &'a mut JoinSet<()>,
    pub(super) local_stats_task: &'a mut Option<tokio::task::JoinHandle<()>>,
}

/// 优雅停机：停掉全部后台任务、断开全部远端并限时等待收尾，不再让任务
/// 随进程退出被粗暴强杀。
///
/// 设计原理（Why）：收尾任务集中在本模块的 JoinSet 里（远端断开、转发
/// 停止），退出前统一限时等待；个别通道关闭挂起不应拖死整个退出。
pub(super) async fn shutdown_all(tables: ShutdownTables<'_>) {
    let ShutdownTables {
        forwards,
        traffic_tasks,
        key_deploy_tasks,
        stats_tasks,
        sftp_transfers,
        remotes,
        cleanup,
        local_stats_task,
    } = tables;
    for (_, handle) in sftp_transfers.drain() {
        handle.abort();
    }
    for (_, handle) in traffic_tasks.drain() {
        handle.abort();
    }
    for (_, handle) in key_deploy_tasks.drain() {
        handle.abort();
    }
    for (_, handle) in stats_tasks.drain() {
        handle.abort();
    }
    for (_, tasks) in forwards.drain() {
        for (_, task) in tasks {
            cleanup.spawn(async move {
                task.stop().await;
            });
        }
    }
    for (_, (_, remote)) in remotes.drain() {
        cleanup.spawn(async move {
            remote.disconnect("application shutdown").await;
        });
    }
    if let Some(task) = local_stats_task.take() {
        task.abort();
    }
    let _ = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while cleanup.join_next().await.is_some() {}
    })
    .await;
}

/// 公钥部署的派发参数。
pub(super) struct KeyDeployParams<'a> {
    pub(super) remotes: &'a mut HashMap<TabId, (ConnectionId, SshRemote)>,
    pub(super) key_deploy_tasks: &'a mut HashMap<TabId, tokio::task::JoinHandle<()>>,
    pub(super) event_tx: &'a flume::Sender<AppEvent>,
    pub(super) tab_id: TabId,
    pub(super) connection_id: ConnectionId,
    pub(super) key_name: String,
    pub(super) public_key: String,
}

/// 公钥部署派发：定位当前连接的远端、停掉旧任务后执行部署并回报结果。
/// 连接未就绪时直接回失败事件。
pub(super) async fn deploy_key(params: KeyDeployParams<'_>) {
    let KeyDeployParams {
        remotes,
        key_deploy_tasks,
        event_tx,
        tab_id,
        connection_id,
        key_name,
        public_key,
    } = params;
    let target = remotes
        .get(&tab_id)
        .filter(|(current, _)| *current == connection_id)
        .map(|(_, remote)| remote.clone());
    match target {
        Some(remote) => {
            stop_key_deploy(key_deploy_tasks, tab_id);
            let event_tx = event_tx.clone();
            let handle = tokio::spawn(async move {
                let error = transom_net::deploy_public_key(&remote, &public_key)
                    .await
                    .err();
                let _ = event_tx
                    .send_async(AppEvent::PublicKeyDeployed {
                        tab_id,
                        connection_id,
                        key_name,
                        error,
                    })
                    .await;
            });
            key_deploy_tasks.insert(tab_id, handle);
        }
        None => {
            let _ = event_tx
                .send_async(AppEvent::PublicKeyDeployed {
                    tab_id,
                    connection_id,
                    key_name,
                    error: Some(AppError::with_detail(
                        ErrorCode::ChannelFailed,
                        "SSH 连接尚未就绪，无法部署公钥",
                    )),
                })
                .await;
        }
    }
}
