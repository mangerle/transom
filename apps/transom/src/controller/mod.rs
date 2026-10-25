use std::collections::HashMap;
use std::sync::Arc;
use tokio::runtime::Runtime;
use tokio::task::JoinSet;
use transom_core::{
    AppCommand, AppError, AppEvent, ConnectionId, ErrorCode, ForwardId, ForwardState,
    ForwardStatus, KnownHostsStore, SecretStore, SettingsStore, SftpOperation, TabId, TerminalIo,
    TransferId,
};
mod host_exec;
mod lifecycle;
mod pty;
mod samplers;
mod sftp_exec;
mod sftp_finish;
mod sync_exec;

use host_exec::run_batch;
use lifecycle::{
    Admission, AdoptConnection, ConnectingTab, KeyDeployParams, ShutdownTables, TabCleanupTables,
    admit_connection, adopt_connection, deploy_key, drop_local_stats, shutdown_all, stop_forwards,
    stop_key_deploy, stop_stats, stop_traffic, stop_zmodem,
};
use pty::ConnectDispatch;
use samplers::{
    run_local_stats_sampler, run_local_traffic_sampler, spawn_remote_stats_sampler,
    spawn_traffic_sampler,
};
use sftp_exec::SftpDispatch;
use sync_exec::{CloudSyncJob, dispatch_cloud_sync};

use transom_net::{
    CloudSyncService, ForwardTask, SftpClient, SshRemote, SshSession, ZmodemConfig, ZmodemControl,
    start_forward,
};

pub struct Controller {
    _runtime: Runtime,
    command_tx: flume::Sender<AppCommand>,
    event_tx: flume::Sender<AppEvent>,
    event_rx: flume::Receiver<AppEvent>,
}

impl Controller {
    pub fn start(
        known_hosts: Arc<dyn KnownHostsStore>,
        secrets: Arc<dyn SecretStore>,
        settings_store: Arc<dyn SettingsStore>,
    ) -> Result<Self, AppError> {
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

        // 云同步服务随控制器构建一次：HTTP 客户端连接池全生命周期复用。
        let cloud_sync = Arc::new(CloudSyncService::new(secrets)?);
        // 事件发送端留一份克隆给应用外壳：深链监听不在控制平面内，
        // 但唤起请求要走同一条事件总线。
        let outer_event_tx = event_tx.clone();
        let root = runtime.spawn(run_controller(
            known_hosts,
            settings_store,
            command_rx,
            internal_tx,
            internal_rx,
            event_tx,
            cloud_sync,
        ));
        // 根控制器是整个应用的控制平面：panic 后命令与事件全部停摆，
        // 必须可观察（日志上报）而不是静默失效。
        runtime.spawn(async move {
            match root.await {
                Ok(()) => log::warn!("控制器主循环已退出"),
                Err(error) if error.is_panic() => {
                    log::error!("控制器主循环异常崩溃，控制平面已失效: {error}")
                }
                Err(error) => log::error!("控制器主循环任务失败，控制平面已失效: {error}"),
            }
        });

        Ok(Self {
            _runtime: runtime,
            command_tx,
            event_tx: outer_event_tx,
            event_rx,
        })
    }

    pub fn command_sender(&self) -> flume::Sender<AppCommand> {
        self.command_tx.clone()
    }

    pub fn event_receiver(&self) -> flume::Receiver<AppEvent> {
        self.event_rx.clone()
    }

    /// 事件总线发送端：供应用外壳（深链监听）投递与连接无关的界面事件。
    pub fn event_sender(&self) -> flume::Sender<AppEvent> {
        self.event_tx.clone()
    }

    /// 控制平面运行时句柄：应用外壳的辅助常驻任务（深链监听）挂到这里，
    /// 与控制器共享线程池并随控制器一起停机，不再单独开一个运行时。
    pub fn runtime(&self) -> &Runtime {
        &self._runtime
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
    TelnetConnected {
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

/// 连接接入时读取 ZMODEM 配置：每次连接取最新设置（改目录后新连接生效），
/// 读取失败按默认值降级（下载目录退回系统下载目录、自动接收开启）。
fn zmodem_config(settings: &Arc<dyn SettingsStore>) -> ZmodemConfig {
    let settings = settings.load().unwrap_or_default();
    ZmodemConfig {
        download_dir: settings.zmodem.resolved_download_dir(),
        auto_receive: settings.zmodem.auto_receive,
    }
}

async fn run_controller(
    known_hosts: Arc<dyn KnownHostsStore>,
    settings_store: Arc<dyn SettingsStore>,
    command_rx: flume::Receiver<AppCommand>,
    internal_tx: flume::Sender<Internal>,
    internal_rx: flume::Receiver<Internal>,
    event_tx: flume::Sender<AppEvent>,
    cloud_sync: Arc<CloudSyncService>,
) {
    let mut current_connections: HashMap<TabId, ConnectionId> = HashMap::new();
    let mut remotes: HashMap<TabId, (ConnectionId, SshRemote)> = HashMap::new();
    let mut sftp_clients: HashMap<TabId, SftpClient> = HashMap::new();
    let mut sftp_tasks: HashMap<(TabId, TransferId), tokio::task::JoinHandle<()>> = HashMap::new();
    let mut forwards: HashMap<TabId, HashMap<ForwardId, ForwardTask>> = HashMap::new();
    let mut traffic_tasks: HashMap<TabId, tokio::task::JoinHandle<()>> = HashMap::new();
    let mut key_deploy_tasks: HashMap<TabId, tokio::task::JoinHandle<()>> = HashMap::new();
    // 远端系统统计采样任务：按标签跟踪，断开/关闭时中止，杜绝孤儿任务。
    let mut stats_tasks: HashMap<TabId, tokio::task::JoinHandle<()>> = HashMap::new();
    // 云同步任务：全局同一时刻只允许一个在途，进行中时拒绝新命令。
    let mut sync_task: Option<tokio::task::JoinHandle<()>> = None;
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
    // 本地网络流量同样是机器级属性：共享采样按机器计一份读取，逐标签广播。
    let local_traffic_tabs: samplers::LocalTrafficTabs =
        Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut local_traffic_task: Option<tokio::task::JoinHandle<()>> = None;
    // ZMODEM：每标签一个事件转发任务与一个控制句柄；连接接入时建立，
    // 断开/关标签/重连时随 stop_zmodem 回收。
    let mut zmodem_tasks: HashMap<TabId, tokio::task::JoinHandle<()>> = HashMap::new();
    let mut zmodem_controls: HashMap<TabId, ZmodemControl> = HashMap::new();
    // 主机动作（拨测/唤醒）批次任务与跨批次并发闸门：信号量放在控制器侧，
    // 用户连点两次也不会翻倍占用在途连接数。
    let action_permits = Arc::new(tokio::sync::Semaphore::new(
        host_exec::MAX_CONCURRENT_ACTIONS,
    ));
    let mut host_action_tasks: JoinSet<()> = JoinSet::new();

    loop {
        let incoming = tokio::select! {
            command = command_rx.recv_async() => Incoming::Command(command),
            internal = internal_rx.recv_async() => Incoming::Internal(internal),
            // 批次任务收自己：异常退出要可见，否则界面会停在"进行中"。
            joined = host_action_tasks.join_next(), if !host_action_tasks.is_empty() => {
                // 批次任务内部已为每条动作补齐结果，这里只记录批次级异常。
                if let Some(Err(error)) = joined {
                    log::error!("主机动作批次任务异常退出，本批结果可能不完整: {error}");
                }
                continue;
            }
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
                    sftp_tasks: &mut sftp_tasks,
                    event_tx: event_tx.clone(),
                    internal_tx: internal_tx.clone(),
                    tab_id,
                    connection_id,
                    op_id,
                    op,
                });
            }
            Incoming::Command(Ok(AppCommand::SftpCancel { tab_id, op_id })) => {
                if let Some(handle) = sftp_tasks.remove(&(tab_id, op_id)) {
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
            Incoming::Command(Ok(AppCommand::CloudSyncPush { settings, catalog })) => {
                dispatch_cloud_sync(
                    &cloud_sync,
                    &event_tx,
                    &mut sync_task,
                    CloudSyncJob::Push { settings, catalog },
                )
                .await;
            }
            Incoming::Command(Ok(AppCommand::CloudSyncPull { settings })) => {
                dispatch_cloud_sync(
                    &cloud_sync,
                    &event_tx,
                    &mut sync_task,
                    CloudSyncJob::Pull { settings },
                )
                .await;
            }
            Incoming::Command(Ok(AppCommand::RunHostActions { actions })) => {
                host_action_tasks.spawn(run_batch(
                    actions,
                    event_tx.clone(),
                    action_permits.clone(),
                ));
            }
            Incoming::Command(Ok(AppCommand::ZmodemUpload { tab_id, files })) => {
                if let Some(control) = zmodem_controls.get(&tab_id)
                    && !control.upload(files)
                {
                    // 通道满或无等待会话：丢弃并让界面提示重试。
                    log::warn!("ZMODEM 上传决策被丢弃，标签 {tab_id:?} 无等待中的传输会话");
                }
            }
            Incoming::Command(Ok(AppCommand::ZmodemCancel { tab_id })) => {
                if let Some(control) = zmodem_controls.get(&tab_id) {
                    control.cancel();
                }
            }
            Incoming::Command(Ok(AppCommand::CloseTab { tab_id })) => {
                current_connections.remove(&tab_id);
                // 关闭即注销连接意图并取消建连任务：DNS/握手/认证等待中的
                // 连接随任务中止，迟到的完成结果被接入裁决拒收
                if let Some(entry) = connecting.remove(&tab_id) {
                    entry.task.abort();
                }
                drop_local_stats(&local_traffic_tabs, tab_id);
                stop_forwards(&mut forwards, &mut cleanup_set, tab_id);
                stop_traffic(&mut traffic_tasks, tab_id);
                stop_key_deploy(&mut key_deploy_tasks, tab_id);
                stop_stats(&mut stats_tasks, tab_id);
                drop_local_stats(&local_stats_tabs, tab_id);
                sftp_clients.remove(&tab_id);
                sftp_tasks.retain(|(t, _), handle| {
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
                // 在途云同步没有可续传的事务，停机即中止。
                if let Some(task) = sync_task.take() {
                    task.abort();
                }
                shutdown_all(ShutdownTables {
                    connecting: &mut connecting,
                    forwards: &mut forwards,
                    traffic_tasks: &mut traffic_tasks,
                    key_deploy_tasks: &mut key_deploy_tasks,
                    stats_tasks: &mut stats_tasks,
                    sftp_tasks: &mut sftp_tasks,
                    remotes: &mut remotes,
                    cleanup: &mut cleanup_set,
                    local_stats_task: &mut local_stats_task,
                    local_traffic_task: &mut local_traffic_task,
                    zmodem_tasks: &mut zmodem_tasks,
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
                    log::debug!(
                        "拒收过期的 SSH 连接完成结果并断开，窗格: {tab_id:?}，连接: {connection_id:?}"
                    );
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
                        zmodem_tasks: &mut zmodem_tasks,
                        zmodem_controls: &mut zmodem_controls,
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
                        zmodem_config: zmodem_config(&settings_store),
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
                    log::debug!(
                        "拒收过期的本地终端连接完成结果，窗格: {tab_id:?}，连接: {connection_id:?}"
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
                        zmodem_tasks: &mut zmodem_tasks,
                        zmodem_controls: &mut zmodem_controls,
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
                        zmodem_config: zmodem_config(&settings_store),
                    },
                    &event_tx,
                    &internal_tx,
                )
                .await;

                // 登记共享订阅：系统统计与网络流量都是机器级属性，各跑一份
                // 采集（多标签各自采样会在本地终端一多时变成每秒 N 份
                // /proc 读取与 N 个 df 子进程）。
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
                if let Ok(mut tabs) = local_traffic_tabs.lock() {
                    tabs.retain(|(tab, _)| *tab != tab_id);
                    tabs.push((tab_id, connection_id));
                }
                if local_traffic_task
                    .as_ref()
                    .is_none_or(|task| task.is_finished())
                {
                    local_traffic_task = Some(tokio::spawn(run_local_traffic_sampler(
                        event_tx.clone(),
                        local_traffic_tabs.clone(),
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
                    log::debug!(
                        "拒收过期的串口连接完成结果，窗格: {tab_id:?}，连接: {connection_id:?}"
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
                        zmodem_tasks: &mut zmodem_tasks,
                        zmodem_controls: &mut zmodem_controls,
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
                        zmodem_config: zmodem_config(&settings_store),
                    },
                    &event_tx,
                    &internal_tx,
                )
                .await;
            }
            Incoming::Internal(Ok(Internal::TelnetConnected {
                tab_id,
                connection_id,
                io,
                join,
            })) => {
                if matches!(
                    admit_connection(&connecting, tab_id, connection_id),
                    Admission::Reject
                ) {
                    // 迟到或已作废的连接：丢弃 IO 即关闭 TCP 流，远端随之
                    // 读到 EOF，退出通知因连接不再现任而被忽略
                    log::debug!(
                        "拒收过期的 Telnet 连接完成结果，窗格: {tab_id:?}，连接: {connection_id:?}"
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
                        zmodem_tasks: &mut zmodem_tasks,
                        zmodem_controls: &mut zmodem_controls,
                    },
                    AdoptConnection {
                        tab_id,
                        connection_id,
                        io,
                        join: async move {
                            let _ = join.await;
                        },
                        remote: None,
                        disconnect_reason: "reconnected to telnet",
                        zmodem_config: zmodem_config(&settings_store),
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
                            &mut sftp_tasks,
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
                    drop_local_stats(&local_traffic_tabs, tab_id);
                    stop_zmodem(&mut zmodem_tasks, &mut zmodem_controls, tab_id);
                    // 优雅断开远端：直接丢弃 Handle 会跳过 disconnect 通知，
                    // 服务器侧要等 keepalive 超时才回收会话。
                    if let Some((_, remote)) = remotes.remove(&tab_id) {
                        cleanup_set.spawn(async move {
                            remote.disconnect("connection lost").await;
                        });
                    }
                    sftp_clients.remove(&tab_id);
                    sftp_tasks.retain(|(t, _), handle| {
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
