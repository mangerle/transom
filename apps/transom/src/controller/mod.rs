use std::collections::HashMap;
use std::sync::Arc;
use tokio::runtime::Runtime;
use tokio::task::JoinSet;
use transom_core::{
    AppCommand, AppError, AppEvent, ConnectionId, ErrorCode, ForwardId, ForwardState,
    ForwardStatus, KnownHostsStore, SftpOperation, TabId, TerminalIo, TransferId,
};
mod lifecycle;
mod pty;
mod samplers;
mod sftp_exec;
mod sftp_finish;

use lifecycle::{
    Admission, AdoptConnection, ConnectingTab, KeyDeployParams, ShutdownTables, TabCleanupTables,
    admit_connection, adopt_connection, deploy_key, drop_local_stats, shutdown_all, stop_forwards,
    stop_key_deploy, stop_stats, stop_traffic,
};
use pty::ConnectDispatch;
use samplers::{
    run_local_stats_sampler, spawn_local_traffic_sampler, spawn_remote_stats_sampler,
    spawn_traffic_sampler,
};
use sftp_exec::SftpDispatch;

use transom_net::{ForwardTask, SftpClient, SshRemote, SshSession, start_forward};

pub struct Controller {
    _runtime: Runtime,
    command_tx: flume::Sender<AppCommand>,
    event_rx: flume::Receiver<AppEvent>,
}

impl Controller {
    pub fn start(known_hosts: Arc<dyn KnownHostsStore>) -> Result<Self, AppError> {
        // 三条通道的容量与背压策略（全工程禁止无界通道）：
        // - 命令 256：用户命令低频，UI 侧 try_send 快速失败并提示；
        // - 事件 1024：进度节流后 10Hz/任务，controller 侧 send_async 反压——
        //   UI 事件循环停滞时拖慢传输而非内存无限积压；
        // - 内部 64：连接完成/断开通知，低频。
        let (command_tx, command_rx) = flume::bounded::<AppCommand>(256);
        let (event_tx, event_rx) = flume::bounded::<AppEvent>(1024);
        let (internal_tx, internal_rx) = flume::bounded::<Internal>(64);
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|error| AppError::with_detail(ErrorCode::Io, error.to_string()))?;

        runtime.spawn(run_controller(
            known_hosts,
            command_rx,
            internal_tx,
            internal_rx,
            event_tx,
        ));

        Ok(Self {
            _runtime: runtime,
            command_tx,
            event_rx,
        })
    }

    pub fn command_sender(&self) -> flume::Sender<AppCommand> {
        self.command_tx.clone()
    }

    pub fn event_receiver(&self) -> flume::Receiver<AppEvent> {
        self.event_rx.clone()
    }
}

enum Internal {
    Connected {
        tab_id: TabId,
        connection_id: ConnectionId,
        session: SshSession,
    },
    LocalConnected {
        tab_id: TabId,
        connection_id: ConnectionId,
        io: TerminalIo,
        join: tokio::sync::oneshot::Receiver<()>,
    },
    SerialConnected {
        tab_id: TabId,
        connection_id: ConnectionId,
        io: TerminalIo,
        join: tokio::sync::oneshot::Receiver<()>,
    },
    Disconnected {
        tab_id: TabId,
        connection_id: ConnectionId,
    },
    /// SFTP 客户端在任务内完成初始化后的提交：主循环校验连接归属后
    /// 才写入客户端缓存并派发操作，分发循环本身不为打开客户端等待。
    SftpReady {
        tab_id: TabId,
        connection_id: ConnectionId,
        op_id: TransferId,
        op: SftpOperation,
        result: Result<SftpClient, AppError>,
    },
    /// 端口转发在任务内建立后的提交：主循环校验连接归属后登记任务
    /// 并向界面回报状态。
    ForwardReady {
        tab_id: TabId,
        connection_id: ConnectionId,
        rule_id: ForwardId,
        result: Result<ForwardTask, AppError>,
    },
}

enum Incoming {
    Command(Result<AppCommand, flume::RecvError>),
    Internal(Result<Internal, flume::RecvError>),
}

async fn run_controller(
    known_hosts: Arc<dyn KnownHostsStore>,
    command_rx: flume::Receiver<AppCommand>,
    internal_tx: flume::Sender<Internal>,
    internal_rx: flume::Receiver<Internal>,
    event_tx: flume::Sender<AppEvent>,
) {
    let mut current_connections: HashMap<TabId, ConnectionId> = HashMap::new();
    let mut remotes: HashMap<TabId, (ConnectionId, SshRemote)> = HashMap::new();
    let mut sftp_clients: HashMap<TabId, SftpClient> = HashMap::new();
    let mut sftp_transfers: HashMap<(TabId, TransferId), tokio::task::JoinHandle<()>> =
        HashMap::new();
    let mut forwards: HashMap<TabId, HashMap<ForwardId, ForwardTask>> = HashMap::new();
    let mut traffic_tasks: HashMap<TabId, tokio::task::JoinHandle<()>> = HashMap::new();
    let mut key_deploy_tasks: HashMap<TabId, tokio::task::JoinHandle<()>> = HashMap::new();
    // 远端系统统计采样任务：按标签跟踪，断开/关闭时中止，杜绝孤儿任务。
    let mut stats_tasks: HashMap<TabId, tokio::task::JoinHandle<()>> = HashMap::new();
    // 收尾任务集合（断开远端、转发优雅停止）：集中纳管 JoinHandle，
    // 停机时统一限时等待，任务不再脱缰。
    let mut cleanup_set: JoinSet<()> = JoinSet::new();
    // 待连接意图表：发起连接即登记，完成事件必须匹配当前意图才接入；
    // 同标签的新连接会顶掉并取消旧建连任务，迟到的旧结果被拒收。
    let mut connecting: HashMap<TabId, ConnectingTab> = HashMap::new();
    // 本地终端共享一份系统统计采集：/proc 读取与 df 子进程按机器计一次，
    // 而不是每个标签各一份；采样器在首个本地连接时拉起、标签清空后退出。
    let local_stats_tabs: Arc<std::sync::Mutex<Vec<(TabId, ConnectionId)>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut local_stats_task: Option<tokio::task::JoinHandle<()>> = None;

    loop {
        let incoming = tokio::select! {
            command = command_rx.recv_async() => Incoming::Command(command),
            internal = internal_rx.recv_async() => Incoming::Internal(internal),
        };

        match incoming {
            Incoming::Command(Ok(AppCommand::Connect {
                tab_id,
                connection_id,
                session,
                size,
                secrets,
            })) => {
                // 发起连接即登记最新意图：同标签的旧建连任务作废并取消，
                // 它迟到的完成结果会被接入裁决拒收，不再覆盖新连接。
                if let Some(previous) = connecting.remove(&tab_id) {
                    previous.task.abort();
                }
                let task = pty::dispatch_connect(ConnectDispatch {
                    known_hosts: known_hosts.clone(),
                    event_tx: event_tx.clone(),
                    internal_tx: internal_tx.clone(),
                    tab_id,
                    connection_id,
                    session,
                    size,
                    secrets,
                });
                connecting.insert(
                    tab_id,
                    ConnectingTab {
                        connection_id,
                        task,
                    },
                );
            }
            Incoming::Command(Ok(AppCommand::Sftp {
                tab_id,
                connection_id,
                op_id,
                op,
            })) => {
                // 分发循环只做快查与派发：客户端初始化在任务内进行，
                // 结果经 SftpReady 回流提交，慢打开不再阻塞其他标签
                sftp_exec::dispatch_sftp_op(SftpDispatch {
                    remotes: &remotes,
                    sftp_clients: &mut sftp_clients,
                    sftp_transfers: &mut sftp_transfers,
                    event_tx: event_tx.clone(),
                    internal_tx: internal_tx.clone(),
                    tab_id,
                    connection_id,
                    op_id,
                    op,
                });
            }
            Incoming::Command(Ok(AppCommand::SftpCancel { tab_id, op_id })) => {
                if let Some(handle) = sftp_transfers.remove(&(tab_id, op_id)) {
                    handle.abort();
                }
            }
            Incoming::Command(Ok(AppCommand::ForwardStart {
                tab_id,
                connection_id,
                rule,
            })) => {
                let Some((current_connection, remote)) = remotes.get(&tab_id).cloned() else {
                    let _ = event_tx
                        .send_async(AppEvent::ForwardChanged {
                            tab_id,
                            connection_id,
                            status: ForwardStatus {
                                rule_id: rule.id,
                                state: ForwardState::Failed,
                                error: Some(AppError::with_detail(
                                    ErrorCode::ForwardFailed,
                                    "SSH connection is not ready",
                                )),
                            },
                        })
                        .await;
                    continue;
                };
                if current_connection != connection_id {
                    continue;
                }
                // 建立转发有多次网络往返，移入任务执行；结果经
                // ForwardReady 回流提交，分发循环不再等待远端应答
                let remote = remote.clone();
                let internal_tx = internal_tx.clone();
                cleanup_set.spawn(async move {
                    let result = start_forward(&remote, &rule).await;
                    let _ = internal_tx
                        .send_async(Internal::ForwardReady {
                            tab_id,
                            connection_id,
                            rule_id: rule.id,
                            result,
                        })
                        .await;
                });
            }
            Incoming::Command(Ok(AppCommand::ForwardStop {
                tab_id,
                connection_id,
                rule_id,
            })) => {
                let Some((current_connection, _)) = remotes.get(&tab_id) else {
                    continue;
                };
                if *current_connection != connection_id {
                    continue;
                }
                if let Some(tasks) = forwards.get_mut(&tab_id)
                    && let Some(task) = tasks.remove(&rule_id)
                {
                    tokio::spawn(async move {
                        task.stop().await;
                    });
                }
                let _ = event_tx
                    .send_async(AppEvent::ForwardChanged {
                        tab_id,
                        connection_id,
                        status: ForwardStatus {
                            rule_id,
                            state: ForwardState::Stopped,
                            error: None,
                        },
                    })
                    .await;
            }
            Incoming::Command(Ok(AppCommand::DeployPublicKey {
                tab_id,
                connection_id,
                key_name,
                public_key,
            })) => {
                deploy_key(KeyDeployParams {
                    remotes: &mut remotes,
                    key_deploy_tasks: &mut key_deploy_tasks,
                    event_tx: &event_tx,
                    tab_id,
                    connection_id,
                    key_name,
                    public_key,
                })
                .await;
            }
            Incoming::Command(Ok(AppCommand::CloseTab { tab_id })) => {
                current_connections.remove(&tab_id);
                // 关闭即注销连接意图并取消建连任务：DNS/握手/认证等待中的
                // 连接随任务中止，迟到的完成结果被接入裁决拒收
                if let Some(entry) = connecting.remove(&tab_id) {
                    entry.task.abort();
                }
                stop_forwards(&mut forwards, &mut cleanup_set, tab_id);
                stop_traffic(&mut traffic_tasks, tab_id);
                stop_key_deploy(&mut key_deploy_tasks, tab_id);
                stop_stats(&mut stats_tasks, tab_id);
                drop_local_stats(&local_stats_tabs, tab_id);
                sftp_clients.remove(&tab_id);
                sftp_transfers.retain(|(t, _), handle| {
                    if *t == tab_id {
                        handle.abort();
                        false
                    } else {
                        true
                    }
                });
                if let Some((_, remote)) = remotes.remove(&tab_id) {
                    cleanup_set.spawn(async move {
                        remote.disconnect("tab closed").await;
                    });
                }
            }
            Incoming::Command(Ok(AppCommand::Shutdown))
            | Incoming::Command(Err(_))
            | Incoming::Internal(Err(_)) => {
                shutdown_all(ShutdownTables {
                    connecting: &mut connecting,
                    forwards: &mut forwards,
                    traffic_tasks: &mut traffic_tasks,
                    key_deploy_tasks: &mut key_deploy_tasks,
                    stats_tasks: &mut stats_tasks,
                    sftp_transfers: &mut sftp_transfers,
                    remotes: &mut remotes,
                    cleanup: &mut cleanup_set,
                    local_stats_task: &mut local_stats_task,
                })
                .await;
                break;
            }
            Incoming::Internal(Ok(Internal::Connected {
                tab_id,
                connection_id,
                session,
            })) => {
                let SshSession {
                    io,
                    join,
                    remote,
                    traffic,
                } = session;
                if matches!(
                    admit_connection(&connecting, tab_id, connection_id),
                    Admission::Reject
                ) {
                    // 迟到或已作废的连接结果：主动断开释放，不触碰任何状态表，
                    // 避免踢掉已接入的新连接
                    tracing::debug!(?tab_id, ?connection_id, "拒收过期的 SSH 连接完成结果并断开");
                    cleanup_set.spawn(async move {
                        remote.disconnect("superseded by newer connection").await;
                    });
                    continue;
                }
                connecting.remove(&tab_id);
                let remote_clone = remote.clone();
                adopt_connection(
                    TabCleanupTables {
                        current_connections: &mut current_connections,
                        forwards: &mut forwards,
                        traffic_tasks: &mut traffic_tasks,
                        key_deploy_tasks: &mut key_deploy_tasks,
                        stats_tasks: &mut stats_tasks,
                        sftp_clients: &mut sftp_clients,
                        remotes: &mut remotes,
                        cleanup: &mut cleanup_set,
                    },
                    AdoptConnection {
                        tab_id,
                        connection_id,
                        io,
                        join: async move {
                            let _ = join.await;
                        },
                        remote: Some(remote),
                        disconnect_reason: "reconnected",
                    },
                    &event_tx,
                    &internal_tx,
                )
                .await;

                let traffic_task =
                    spawn_traffic_sampler(traffic, tab_id, connection_id, event_tx.clone());
                traffic_tasks.insert(tab_id, traffic_task);

                let stats_task = spawn_remote_stats_sampler(
                    remote_clone,
                    tab_id,
                    connection_id,
                    event_tx.clone(),
                );
                stats_tasks.insert(tab_id, stats_task);
            }
            Incoming::Internal(Ok(Internal::LocalConnected {
                tab_id,
                connection_id,
                io,
                join,
            })) => {
                if matches!(
                    admit_connection(&connecting, tab_id, connection_id),
                    Admission::Reject
                ) {
                    // 迟到或已作废的连接：丢弃 IO 即关闭 PTY 主端/串口句柄，
                    // 子进程随之退出，退出通知因连接不再现任而被忽略
                    tracing::debug!(
                        ?tab_id,
                        ?connection_id,
                        "拒收过期的LocalConnected连接完成结果"
                    );
                    continue;
                }
                connecting.remove(&tab_id);
                adopt_connection(
                    TabCleanupTables {
                        current_connections: &mut current_connections,
                        forwards: &mut forwards,
                        traffic_tasks: &mut traffic_tasks,
                        key_deploy_tasks: &mut key_deploy_tasks,
                        stats_tasks: &mut stats_tasks,
                        sftp_clients: &mut sftp_clients,
                        remotes: &mut remotes,
                        cleanup: &mut cleanup_set,
                    },
                    AdoptConnection {
                        tab_id,
                        connection_id,
                        io,
                        join: async move {
                            let _ = join.await;
                        },
                        remote: None,
                        disconnect_reason: "reconnected to local",
                    },
                    &event_tx,
                    &internal_tx,
                )
                .await;

                let traffic_task =
                    spawn_local_traffic_sampler(tab_id, connection_id, event_tx.clone());
                traffic_tasks.insert(tab_id, traffic_task);

                // 登记共享统计订阅：采集本体见 run_local_stats_sampler，
                // 每台机器只跑一份（多标签各自 spawn df 会在本地终端一多时
                // 变成每 3 秒 N 个子进程）。
                if let Ok(mut tabs) = local_stats_tabs.lock() {
                    tabs.retain(|(tab, _)| *tab != tab_id);
                    tabs.push((tab_id, connection_id));
                }
                if local_stats_task
                    .as_ref()
                    .is_none_or(|task| task.is_finished())
                {
                    local_stats_task = Some(tokio::spawn(run_local_stats_sampler(
                        event_tx.clone(),
                        local_stats_tabs.clone(),
                    )));
                }
            }
            Incoming::Internal(Ok(Internal::SerialConnected {
                tab_id,
                connection_id,
                io,
                join,
            })) => {
                if matches!(
                    admit_connection(&connecting, tab_id, connection_id),
                    Admission::Reject
                ) {
                    // 迟到或已作废的连接：丢弃 IO 即关闭 PTY 主端/串口句柄，
                    // 子进程随之退出，退出通知因连接不再现任而被忽略
                    tracing::debug!(
                        ?tab_id,
                        ?connection_id,
                        "拒收过期的SerialConnected连接完成结果"
                    );
                    continue;
                }
                connecting.remove(&tab_id);
                adopt_connection(
                    TabCleanupTables {
                        current_connections: &mut current_connections,
                        forwards: &mut forwards,
                        traffic_tasks: &mut traffic_tasks,
                        key_deploy_tasks: &mut key_deploy_tasks,
                        stats_tasks: &mut stats_tasks,
                        sftp_clients: &mut sftp_clients,
                        remotes: &mut remotes,
                        cleanup: &mut cleanup_set,
                    },
                    AdoptConnection {
                        tab_id,
                        connection_id,
                        io,
                        join: async move {
                            let _ = join.await;
                        },
                        remote: None,
                        disconnect_reason: "reconnected to serial",
                    },
                    &event_tx,
                    &internal_tx,
                )
                .await;
            }
            Incoming::Internal(Ok(Internal::SftpReady {
                tab_id,
                connection_id,
                op_id,
                op,
                result,
            })) => {
                // 提交前校验归属：标签关闭或连接已被替换时丢弃客户端，
                // 迟到的初始化结果不得重新登记资源
                if current_connections.get(&tab_id) != Some(&connection_id) {
                    drop(result);
                    continue;
                }
                match result {
                    Ok(client) => {
                        sftp_clients.insert(tab_id, client.clone());
                        sftp_exec::spawn_sftp_op(
                            &mut sftp_transfers,
                            event_tx.clone(),
                            tab_id,
                            connection_id,
                            op_id,
                            client,
                            op,
                        );
                    }
                    Err(error) => {
                        let _ = event_tx
                            .send_async(AppEvent::SftpFailed {
                                tab_id,
                                connection_id,
                                op_id,
                                error,
                            })
                            .await;
                    }
                }
            }
            Incoming::Internal(Ok(Internal::ForwardReady {
                tab_id,
                connection_id,
                rule_id,
                result,
            })) => {
                let is_current = remotes
                    .get(&tab_id)
                    .is_some_and(|(current, _)| *current == connection_id);
                let status = match (is_current, result) {
                    (false, Ok(task)) => {
                        // 连接已失效：立即停止刚建立的转发并释放
                        cleanup_set.spawn(async move {
                            task.stop().await;
                        });
                        ForwardStatus {
                            rule_id,
                            state: ForwardState::Stopped,
                            error: None,
                        }
                    }
                    (false, Err(_)) => continue,
                    (true, Ok(task)) => {
                        let bound_port = task.bound_port;
                        forwards.entry(tab_id).or_default().insert(rule_id, task);
                        ForwardStatus {
                            rule_id,
                            state: ForwardState::Active { bound_port },
                            error: None,
                        }
                    }
                    (true, Err(error)) => ForwardStatus {
                        rule_id,
                        state: ForwardState::Failed,
                        error: Some(error),
                    },
                };
                let _ = event_tx
                    .send_async(AppEvent::ForwardChanged {
                        tab_id,
                        connection_id,
                        status,
                    })
                    .await;
            }
            Incoming::Internal(Ok(Internal::Disconnected {
                tab_id,
                connection_id,
            })) => {
                let is_current = current_connections.get(&tab_id) == Some(&connection_id);
                if is_current {
                    current_connections.remove(&tab_id);
                    stop_forwards(&mut forwards, &mut cleanup_set, tab_id);
                    stop_traffic(&mut traffic_tasks, tab_id);
                    stop_key_deploy(&mut key_deploy_tasks, tab_id);
                    stop_stats(&mut stats_tasks, tab_id);
                    drop_local_stats(&local_stats_tabs, tab_id);
                    // 优雅断开远端：直接丢弃 Handle 会跳过 disconnect 通知，
                    // 服务器侧要等 keepalive 超时才回收会话。
                    if let Some((_, remote)) = remotes.remove(&tab_id) {
                        cleanup_set.spawn(async move {
                            remote.disconnect("connection lost").await;
                        });
                    }
                    sftp_clients.remove(&tab_id);
                    sftp_transfers.retain(|(t, _), handle| {
                        if *t == tab_id {
                            handle.abort();
                            false
                        } else {
                            true
                        }
                    });
                    let _ = event_tx
                        .send_async(AppEvent::Disconnected {
                            tab_id,
                            connection_id,
                        })
                        .await;
                }
            }
        }
    }
}
