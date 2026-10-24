use async_trait::async_trait;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tokio::process::Command;
use tokio::runtime::Runtime;
use transom_core::traffic::parse_net_dev_totals;
use transom_core::{
    AppCommand, AppError, AppEvent, Connection, ConnectionId, ErrorCode, ForwardId, ForwardState,
    ForwardStatus, KnownHostsStore, NetworkTraffic, PromptInfo, SftpOperation, TabId, TerminalIo,
    TrafficCounter, TransferId,
};
use transom_net::{
    AuthPrompter, ForwardTask, SftpClient, SshRemote, SshSession, connect_ssh, deploy_public_key,
    open_serial, start_forward,
};

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
    // 本地终端共享一份系统统计采集：/proc 读取与 df 子进程按机器计一次，
    // 而不是每个标签各一份；采样器在首个本地连接时拉起、标签清空后退出。
    let local_stats_tabs: Arc<std::sync::Mutex<Vec<(TabId, ConnectionId)>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut local_stats_task: Option<tokio::task::JoinHandle<()>> = None;

    /// 注销一个标签的本地统计订阅；订阅清空后采样器自行退出。
    fn drop_local_stats(
        local_stats_tabs: &Arc<std::sync::Mutex<Vec<(TabId, ConnectionId)>>>,
        tab_id: TabId,
    ) {
        if let Ok(mut tabs) = local_stats_tabs.lock() {
            tabs.retain(|(tab, _)| *tab != tab_id);
        }
    }

    fn stop_forwards(
        forwards: &mut HashMap<TabId, HashMap<ForwardId, ForwardTask>>,
        tab_id: TabId,
    ) {
        if let Some(tasks) = forwards.remove(&tab_id) {
            for (_, task) in tasks {
                tokio::spawn(async move {
                    task.stop().await;
                });
            }
        }
    }

    fn stop_traffic(
        traffic_tasks: &mut HashMap<TabId, tokio::task::JoinHandle<()>>,
        tab_id: TabId,
    ) {
        if let Some(task) = traffic_tasks.remove(&tab_id) {
            task.abort();
        }
    }

    fn stop_key_deploy(
        key_deploy_tasks: &mut HashMap<TabId, tokio::task::JoinHandle<()>>,
        tab_id: TabId,
    ) {
        if let Some(task) = key_deploy_tasks.remove(&tab_id) {
            task.abort();
        }
    }

    fn spawn_traffic_sampler(
        traffic: Arc<TrafficCounter>,
        tab_id: TabId,
        connection_id: ConnectionId,
        event_tx: flume::Sender<AppEvent>,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            // 先交一个零样本，让网络徽标在连接建立后立刻出现；首个真实速率
            // 仍从完整的一秒窗口开始计算，避免瞬时大数字。
            if event_tx
                .send_async(AppEvent::NetworkTrafficUpdated {
                    tab_id,
                    connection_id,
                    traffic: NetworkTraffic::default(),
                })
                .await
                .is_err()
            {
                return;
            }
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            // 跳过 interval 的第一个瞬时 tick，保证首个速率窗口是完整的 1 秒。
            interval.tick().await;
            let mut previous = traffic.totals();
            let mut previous_at = std::time::Instant::now();
            loop {
                interval.tick().await;
                let now = std::time::Instant::now();
                let elapsed = now.saturating_duration_since(previous_at);
                previous_at = now;
                let current = traffic.totals();
                let sample = NetworkTraffic::from_delta(previous, current, elapsed);
                previous = current;
                if event_tx
                    .send_async(AppEvent::NetworkTrafficUpdated {
                        tab_id,
                        connection_id,
                        traffic: sample,
                    })
                    .await
                    .is_err()
                {
                    break;
                }
            }
        })
    }

    /// 本地终端的网络采样：读取本机 `/proc/net/dev` 的非回环网卡累计字节。
    ///
    /// 与 SSH 传输层计数不同，这里得到的是整台机器所有非回环接口的合计吞吐，
    /// 因此界面上要按会话类型区分提示文案。读取失败时跳过本轮并保留上一次
    /// 有效样本，避免把一次文件读取抖动显示成断网。
    fn spawn_local_traffic_sampler(
        tab_id: TabId,
        connection_id: ConnectionId,
        event_tx: flume::Sender<AppEvent>,
    ) -> tokio::task::JoinHandle<()> {
        async fn read_totals() -> Option<(u64, u64)> {
            let content = tokio::fs::read_to_string("/proc/net/dev").await.ok()?;
            parse_net_dev_totals(&content)
        }

        tokio::spawn(async move {
            if event_tx
                .send_async(AppEvent::NetworkTrafficUpdated {
                    tab_id,
                    connection_id,
                    traffic: NetworkTraffic::default(),
                })
                .await
                .is_err()
            {
                return;
            }
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            interval.tick().await;
            let mut previous = read_totals().await;
            let mut previous_at = std::time::Instant::now();
            loop {
                interval.tick().await;
                let Some(current) = read_totals().await else {
                    continue;
                };
                let now = std::time::Instant::now();
                let elapsed = now.saturating_duration_since(previous_at);
                previous_at = now;
                let Some(previous_totals) = previous.replace(current) else {
                    continue;
                };
                let sample = NetworkTraffic::from_delta(previous_totals, current, elapsed);
                if event_tx
                    .send_async(AppEvent::NetworkTrafficUpdated {
                        tab_id,
                        connection_id,
                        traffic: sample,
                    })
                    .await
                    .is_err()
                {
                    break;
                }
            }
        })
    }

    enum Incoming {
        Command(Result<AppCommand, flume::RecvError>),
        Internal(Result<Internal, flume::RecvError>),
    }

    /// 本地终端共享的系统统计采样器。
    ///
    /// 设计原理（Why）：
    /// - 实现初衷：统计口径是"整台机器"（CPU/内存/磁盘），多个本地标签
    ///   各自采集只会得到 N 份相同数据，却付出 N 倍 /proc 读取与 df 子进程；
    /// - 核心优势：采集按机器计一次，结果广播给全部订阅标签；OS 标识是
    ///   静态信息，首轮读取后缓存，后续轮次不再碰 /etc/os-release；
    /// - 代价与局限：采样周期内新登记的标签要等下一轮才收到数据（3 秒），
    ///   对状态栏指标无感知影响。
    async fn run_local_stats_sampler(
        event_tx: flume::Sender<AppEvent>,
        local_tabs: Arc<std::sync::Mutex<Vec<(TabId, ConnectionId)>>>,
    ) {
        let mut prev_cpu = None;
        let mut cached_os: Option<String> = None;
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(3));
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        // 跳过首个瞬时 tick，保证首个采样窗口是完整的 3 秒。
        interval.tick().await;
        loop {
            // 持锁只做纯内存克隆，不跨 await；订阅清空即退出，
            // 下一个本地连接会重新拉起本采样器。
            let tabs = match local_tabs.lock() {
                Ok(tabs) if tabs.is_empty() => break,
                Ok(tabs) => tabs.clone(),
                Err(_) => break,
            };
            let stat = tokio::fs::read_to_string("/proc/stat")
                .await
                .unwrap_or_default();
            let mem = tokio::fs::read_to_string("/proc/meminfo")
                .await
                .unwrap_or_default();
            let uptime = tokio::fs::read_to_string("/proc/uptime")
                .await
                .unwrap_or_default();
            let os = match &cached_os {
                Some(_) => String::new(),
                None => tokio::fs::read_to_string("/etc/os-release")
                    .await
                    .unwrap_or_default(),
            };
            let df = match Command::new("df").args(["-P", "-k", "/"]).output().await {
                Ok(out) => String::from_utf8_lossy(&out.stdout).to_string(),
                Err(_) => String::new(),
            };
            let output = format!("{stat}\n{mem}\n__UPTIME__\n{uptime}\n__DF__\n{df}\n__OS__\n{os}");
            let (mut stats, curr_cpu) =
                transom_core::system_stats::parse_combined_stats(&output, prev_cpu);
            prev_cpu = curr_cpu;
            match &cached_os {
                Some(cached) => stats.os_name = cached.clone(),
                None => cached_os = Some(stats.os_name.clone()),
            }
            for (tab_id, connection_id) in tabs {
                if event_tx
                    .send_async(AppEvent::SystemStatsUpdated {
                        tab_id,
                        connection_id,
                        stats: stats.clone(),
                    })
                    .await
                    .is_err()
                {
                    return;
                }
            }
            interval.tick().await;
        }
    }

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
                let event_tx = event_tx.clone();
                let internal_tx = internal_tx.clone();
                let known_hosts = known_hosts.clone();
                tokio::spawn(async move {
                    match &session.connection {
                        Connection::Ssh(ssh) => {
                            let prompter = Arc::new(UiPrompter {
                                tab_id,
                                connection_id,
                                event_tx: event_tx.clone(),
                            });
                            match connect_ssh(ssh, size, secrets, prompter, known_hosts).await {
                                Ok(session) => {
                                    let _ = internal_tx
                                        .send_async(Internal::Connected {
                                            tab_id,
                                            connection_id,
                                            session,
                                        })
                                        .await;
                                }
                                Err(error) => {
                                    let _ = event_tx
                                        .send_async(AppEvent::ConnectionFailed {
                                            tab_id,
                                            connection_id,
                                            error,
                                        })
                                        .await;
                                }
                            }
                        }
                        Connection::Local(local) => match open_local_pty(local, size) {
                            Ok((io, join)) => {
                                let _ = internal_tx
                                    .send_async(Internal::LocalConnected {
                                        tab_id,
                                        connection_id,
                                        io,
                                        join,
                                    })
                                    .await;
                            }
                            Err(error) => {
                                let _ = event_tx
                                    .send_async(AppEvent::ConnectionFailed {
                                        tab_id,
                                        connection_id,
                                        error,
                                    })
                                    .await;
                            }
                        },
                        Connection::Serial(serial) => {
                            let serial = serial.clone();
                            let internal_tx = internal_tx.clone();
                            let event_tx = event_tx.clone();
                            // 串口打开是阻塞系统调用，禁止占用异步工作线程。
                            match tokio::task::spawn_blocking(move || open_serial(&serial)).await {
                                Ok(Ok(connected)) => {
                                    let _ = internal_tx
                                        .send_async(Internal::SerialConnected {
                                            tab_id,
                                            connection_id,
                                            io: connected.io,
                                            join: connected.exit,
                                        })
                                        .await;
                                }
                                Ok(Err(error)) => {
                                    let _ = event_tx
                                        .send_async(AppEvent::ConnectionFailed {
                                            tab_id,
                                            connection_id,
                                            error,
                                        })
                                        .await;
                                }
                                Err(error) => {
                                    let _ = event_tx
                                        .send_async(AppEvent::ConnectionFailed {
                                            tab_id,
                                            connection_id,
                                            error: AppError::with_detail(
                                                ErrorCode::Io,
                                                format!("串口连接任务异常终止: {error}"),
                                            ),
                                        })
                                        .await;
                                }
                            }
                        }
                        _ => {
                            let _ = event_tx
                                .send_async(AppEvent::ConnectionFailed {
                                    tab_id,
                                    connection_id,
                                    error: AppError::with_detail(
                                        ErrorCode::Configuration,
                                        "不支持的连接类型",
                                    ),
                                })
                                .await;
                        }
                    }
                });
            }
            Incoming::Command(Ok(AppCommand::Sftp {
                tab_id,
                connection_id,
                op_id,
                op,
            })) => {
                let Some((current_connection, remote)) = remotes.get(&tab_id).cloned() else {
                    let _ = event_tx
                        .send_async(AppEvent::SftpFailed {
                            tab_id,
                            connection_id,
                            op_id,
                            error: AppError::with_detail(
                                ErrorCode::ChannelFailed,
                                "SSH connection is not ready",
                            ),
                        })
                        .await;
                    continue;
                };
                if current_connection != connection_id {
                    let _ = event_tx
                        .send_async(AppEvent::SftpFailed {
                            tab_id,
                            connection_id,
                            op_id,
                            error: AppError::with_detail(
                                ErrorCode::ChannelFailed,
                                "SSH connection changed",
                            ),
                        })
                        .await;
                    continue;
                }

                let client = if let Some(client) = sftp_clients.get(&tab_id) {
                    client.clone()
                } else {
                    match SftpClient::open(&remote).await {
                        Ok(client) => {
                            sftp_clients.insert(tab_id, client.clone());
                            client
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
                            continue;
                        }
                    }
                };

                let is_transfer = matches!(
                    op,
                    SftpOperation::Download { .. }
                        | SftpOperation::Upload { .. }
                        | SftpOperation::DownloadDir { .. }
                        | SftpOperation::UploadDir { .. }
                        | SftpOperation::SyncDirs { .. }
                );
                let event_tx = event_tx.clone();
                // 直接 spawn 具名 async 函数返回的 future：经由 async 块包装会触发
                // rustc 对泛型闭包的 HRTT 推断限制（FnOnce is not general enough）
                let handle = tokio::spawn(execute_sftp_op(
                    event_tx,
                    tab_id,
                    connection_id,
                    op_id,
                    client,
                    op,
                ));
                if is_transfer {
                    sftp_transfers.retain(|_, h| !h.is_finished());
                    sftp_transfers.insert((tab_id, op_id), handle);
                }
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
                let status = match start_forward(&remote, &rule).await {
                    Ok(task) => {
                        let bound_port = task.bound_port;
                        forwards.entry(tab_id).or_default().insert(rule.id, task);
                        ForwardStatus {
                            rule_id: rule.id,
                            state: ForwardState::Active { bound_port },
                            error: None,
                        }
                    }
                    Err(error) => ForwardStatus {
                        rule_id: rule.id,
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
                let target = remotes
                    .get(&tab_id)
                    .filter(|(current, _)| *current == connection_id)
                    .map(|(_, remote)| remote.clone());
                match target {
                    Some(remote) => {
                        stop_key_deploy(&mut key_deploy_tasks, tab_id);
                        let event_tx = event_tx.clone();
                        let handle = tokio::spawn(async move {
                            let error = deploy_public_key(&remote, &public_key).await.err();
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
            Incoming::Command(Ok(AppCommand::CloseTab { tab_id })) => {
                current_connections.remove(&tab_id);
                stop_forwards(&mut forwards, tab_id);
                stop_traffic(&mut traffic_tasks, tab_id);
                stop_key_deploy(&mut key_deploy_tasks, tab_id);
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
                    tokio::spawn(async move {
                        remote.disconnect("tab closed").await;
                    });
                }
            }
            Incoming::Command(Ok(AppCommand::Shutdown))
            | Incoming::Command(Err(_))
            | Incoming::Internal(Err(_)) => break,
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
                current_connections.insert(tab_id, connection_id);
                stop_forwards(&mut forwards, tab_id);
                stop_traffic(&mut traffic_tasks, tab_id);
                stop_key_deploy(&mut key_deploy_tasks, tab_id);
                sftp_clients.remove(&tab_id);
                let remote_clone = remote.clone();
                if let Some((_, old_remote)) = remotes.insert(tab_id, (connection_id, remote)) {
                    tokio::spawn(async move {
                        old_remote.disconnect("reconnected").await;
                    });
                }
                let _ = event_tx
                    .send_async(AppEvent::TerminalReady {
                        tab_id,
                        connection_id,
                        io,
                    })
                    .await;
                let internal_tx = internal_tx.clone();
                tokio::spawn(async move {
                    let _ = join.await;
                    let _ = internal_tx
                        .send_async(Internal::Disconnected {
                            tab_id,
                            connection_id,
                        })
                        .await;
                });

                let traffic_task =
                    spawn_traffic_sampler(traffic, tab_id, connection_id, event_tx.clone());
                traffic_tasks.insert(tab_id, traffic_task);

                let event_tx_stats = event_tx.clone();
                tokio::spawn(async move {
                    let mut prev_cpu = None;
                    let mut interval = tokio::time::interval(std::time::Duration::from_secs(3));
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    // 跳过 interval 的第一个瞬时 tick，保证首个采样窗口是完整的
                    // 3 秒；否则首个 tick 立即完成，前两次采样背靠背，CPU 占比失真。
                    interval.tick().await;
                    // OS 标识是静态信息：仅首轮附带 os-release，之后缓存复用，
                    // 省掉每 3 秒一次重复读取静态文件的开销。
                    let mut cached_os: Option<String> = None;
                    loop {
                        let cmd = if cached_os.is_none() {
                            "cat /proc/stat /proc/meminfo 2>/dev/null; echo \"__UPTIME__\"; cat /proc/uptime 2>/dev/null; echo \"__DF__\"; df -P -k / 2>/dev/null; echo \"__OS__\"; cat /etc/os-release 2>/dev/null"
                        } else {
                            "cat /proc/stat /proc/meminfo 2>/dev/null; echo \"__UPTIME__\"; cat /proc/uptime 2>/dev/null; echo \"__DF__\"; df -P -k / 2>/dev/null"
                        };
                        match remote_clone.exec_command(cmd).await {
                            Ok(output) if !output.is_empty() => {
                                let (mut stats, curr_cpu) =
                                    transom_core::system_stats::parse_combined_stats(
                                        &output, prev_cpu,
                                    );
                                prev_cpu = curr_cpu;
                                match &cached_os {
                                    Some(cached) => stats.os_name = cached.clone(),
                                    None => cached_os = Some(stats.os_name.clone()),
                                }
                                if event_tx_stats
                                    .send_async(AppEvent::SystemStatsUpdated {
                                        tab_id,
                                        connection_id,
                                        stats,
                                    })
                                    .await
                                    .is_err()
                                {
                                    break;
                                }
                            }
                            _ => break,
                        }
                        interval.tick().await;
                    }
                });
            }
            Incoming::Internal(Ok(Internal::LocalConnected {
                tab_id,
                connection_id,
                io,
                join,
            })) => {
                current_connections.insert(tab_id, connection_id);
                stop_forwards(&mut forwards, tab_id);
                stop_traffic(&mut traffic_tasks, tab_id);
                stop_key_deploy(&mut key_deploy_tasks, tab_id);
                sftp_clients.remove(&tab_id);
                if let Some((_, old_remote)) = remotes.remove(&tab_id) {
                    tokio::spawn(async move {
                        old_remote.disconnect("reconnected to local").await;
                    });
                }
                let _ = event_tx
                    .send_async(AppEvent::TerminalReady {
                        tab_id,
                        connection_id,
                        io,
                    })
                    .await;
                let internal_tx = internal_tx.clone();
                tokio::spawn(async move {
                    let _ = join.await;
                    let _ = internal_tx
                        .send_async(Internal::Disconnected {
                            tab_id,
                            connection_id,
                        })
                        .await;
                });

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
                current_connections.insert(tab_id, connection_id);
                stop_forwards(&mut forwards, tab_id);
                stop_traffic(&mut traffic_tasks, tab_id);
                stop_key_deploy(&mut key_deploy_tasks, tab_id);
                sftp_clients.remove(&tab_id);
                if let Some((_, old_remote)) = remotes.remove(&tab_id) {
                    tokio::spawn(async move {
                        old_remote.disconnect("reconnected to serial").await;
                    });
                }
                let _ = event_tx
                    .send_async(AppEvent::TerminalReady {
                        tab_id,
                        connection_id,
                        io,
                    })
                    .await;
                let internal_tx = internal_tx.clone();
                tokio::spawn(async move {
                    let _ = join.await;
                    let _ = internal_tx
                        .send_async(Internal::Disconnected {
                            tab_id,
                            connection_id,
                        })
                        .await;
                });
            }
            Incoming::Internal(Ok(Internal::Disconnected {
                tab_id,
                connection_id,
            })) => {
                let is_current = current_connections.get(&tab_id) == Some(&connection_id);
                if is_current {
                    current_connections.remove(&tab_id);
                    stop_forwards(&mut forwards, tab_id);
                    stop_traffic(&mut traffic_tasks, tab_id);
                    stop_key_deploy(&mut key_deploy_tasks, tab_id);
                    drop_local_stats(&local_stats_tabs, tab_id);
                    remotes.remove(&tab_id);
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

/// 悬停预览读取的最长等待时间。
///
/// 预览属于可随时放弃的低优先级只读请求；远端套接字挂起时必须尽快收尾，
/// 否则一次卡顿会让预览通道长期占用内存并阻塞同一会话上的后续预览。
const PREVIEW_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// 传输进度事件的最小发送间隔。
///
/// 传输层每 64KB 回调一次，内网下每秒可达数千次；直接透传会让 UI 以相同
/// 频率整帧重绘。进度是单调可覆盖的状态量且终值由 `SftpFinished` 事件兜底，
/// 中间样本按 100ms 窗口抽样丢弃不损失最终一致性，人眼对进度条刷新的
/// 分辨率也远低于 10Hz。
const PROGRESS_EVENT_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);

/// 构造按 [`PROGRESS_EVENT_INTERVAL`] 节流的进度回调。
fn throttled_progress(
    progress_tx: flume::Sender<AppEvent>,
    tab_id: TabId,
    connection_id: ConnectionId,
    op_id: TransferId,
) -> impl FnMut(u64, Option<u64>) + Send {
    let mut last_sent: Option<std::time::Instant> = None;
    move |transferred, total| {
        let now = std::time::Instant::now();
        if last_sent.is_none_or(|at| now.duration_since(at) >= PROGRESS_EVENT_INTERVAL) {
            last_sent = Some(now);
            // 回调运行在非 async 上下文：队列满时快速失败丢弃本条样本。
            // 这是显式背压——UI 消费停滞时宁可进度刷新变慢，也不阻塞传输线程。
            let _ = progress_tx.try_send(AppEvent::SftpProgress {
                tab_id,
                connection_id,
                op_id,
                transferred,
                total,
            });
        }
    }
}

/// 编辑器读取远端文本的最长等待时间。
///
/// 与预览不同，编辑是用户主动发起且界面正在等的操作，超时过短会把慢链路上一次
/// 正常读取误判为失败。按内置编辑器的 8 MiB 上限与约 1 Mbps 的慢链路估算需要
/// 60 秒以上，留一倍余量取 2 分钟：仍能兜住"对端挂起导致协程永久泄漏"，
/// 又不至于让用户对着读取中无限等待。
const TEXT_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// 编辑器回传文本的最长等待时间。写入需要远端落盘并回读确认，比读取更慢，取 5 分钟。
const TEXT_WRITE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

async fn execute_sftp_op(
    event_tx: flume::Sender<AppEvent>,
    tab_id: TabId,
    connection_id: ConnectionId,
    op_id: TransferId,
    client: SftpClient,
    op: SftpOperation,
) {
    match op {
        SftpOperation::List { path } => match client.list(&path).await {
            Ok((path, entries)) => {
                let _ = event_tx
                    .send_async(AppEvent::SftpListing {
                        tab_id,
                        connection_id,
                        path,
                        entries,
                    })
                    .await;
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
        },
        SftpOperation::Download {
            remote_path,
            local_path,
            resume,
            verify,
        } => {
            let result = client
                .download_resumable(
                    &remote_path,
                    &local_path,
                    resume,
                    throttled_progress(event_tx.clone(), tab_id, connection_id, op_id),
                )
                .await;
            let result =
                verify_transfer_result(&client, result, verify, &local_path, &remote_path).await;
            finish_sftp(event_tx, tab_id, connection_id, op_id, result).await;
        }
        SftpOperation::Upload {
            local_path,
            remote_path,
            resume,
            verify,
        } => {
            let result = client
                .upload_resumable(
                    &local_path,
                    &remote_path,
                    resume,
                    throttled_progress(event_tx.clone(), tab_id, connection_id, op_id),
                )
                .await;
            let result =
                verify_transfer_result(&client, result, verify, &local_path, &remote_path).await;
            finish_sftp(event_tx, tab_id, connection_id, op_id, result).await;
        }
        SftpOperation::UploadDir {
            local_path,
            remote_path,
            resume,
            filter,
            verify,
        } => {
            let result = client
                .upload_dir(
                    &local_path,
                    &remote_path,
                    resume,
                    &filter,
                    verify,
                    throttled_progress(event_tx.clone(), tab_id, connection_id, op_id),
                )
                .await;
            finish_sftp_summary(event_tx, tab_id, connection_id, op_id, result).await;
        }
        SftpOperation::DownloadDir {
            remote_path,
            local_path,
            resume,
            filter,
            verify,
        } => {
            let result = client
                .download_dir(
                    &remote_path,
                    &local_path,
                    resume,
                    &filter,
                    verify,
                    throttled_progress(event_tx.clone(), tab_id, connection_id, op_id),
                )
                .await;
            finish_sftp_summary(event_tx, tab_id, connection_id, op_id, result).await;
        }
        SftpOperation::Mkdir { path } => {
            let result = client.create_dir(&path).await;
            finish_sftp_none(event_tx, tab_id, connection_id, op_id, result).await;
        }
        SftpOperation::Rename { from, to } => {
            let result = client.rename(&from, &to).await;
            finish_sftp_none(event_tx, tab_id, connection_id, op_id, result).await;
        }
        SftpOperation::Delete { path, is_dir } => {
            let result = client.delete(&path, is_dir).await;
            finish_sftp_none(event_tx, tab_id, connection_id, op_id, result).await;
        }
        SftpOperation::DeleteMany { targets } => {
            let mut result = Ok(());
            for target in targets {
                if let Err(error) = client.delete(&target.path, target.is_dir).await {
                    result = Err(error);
                    break;
                }
            }
            finish_sftp_none(event_tx, tab_id, connection_id, op_id, result).await;
        }
        SftpOperation::Chmod { path, permissions } => {
            let result = client.chmod(&path, permissions).await;
            finish_sftp_none(event_tx, tab_id, connection_id, op_id, result).await;
        }
        SftpOperation::Stat { path } => match client.stat(&path).await {
            Ok(metadata) => {
                let _ = event_tx
                    .send_async(AppEvent::SftpMetadata {
                        tab_id,
                        connection_id,
                        op_id,
                        metadata,
                    })
                    .await;
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
        },
        SftpOperation::Preview {
            path,
            mode,
            max_bytes,
        } => {
            // 预览是低优先级只读请求：远端卡住时超时放弃，避免长时间占用
            // 会话与内存；失败走独立事件，不污染传输队列和状态栏。
            let result =
                tokio::time::timeout(PREVIEW_TIMEOUT, client.read_preview(&path, mode, max_bytes))
                    .await;
            match result {
                Ok(Ok((bytes, truncated))) => {
                    let _ = event_tx
                        .send_async(AppEvent::SftpPreview {
                            tab_id,
                            connection_id,
                            op_id,
                            bytes,
                            truncated,
                        })
                        .await;
                }
                Ok(Err(error)) => {
                    let _ = event_tx
                        .send_async(AppEvent::SftpPreviewFailed {
                            tab_id,
                            connection_id,
                            op_id,
                            error,
                        })
                        .await;
                }
                Err(_) => {
                    let _ = event_tx
                        .send_async(AppEvent::SftpPreviewFailed {
                            tab_id,
                            connection_id,
                            op_id,
                            error: AppError::with_detail(
                                ErrorCode::SftpFailed,
                                "读取远端预览超时（10 秒）",
                            ),
                        })
                        .await;
                }
            }
        }
        SftpOperation::ReadText { path, max_bytes } => {
            // 编辑器取全文：与预览同为独立通道，不产生进度事件、不进传输队列。
            // 超时按"上限字节数 + 慢链路"放宽，既要能兜住永久挂起的任务，
            // 也不能让一次正常的慢速读取被误判失败。
            let result =
                tokio::time::timeout(TEXT_READ_TIMEOUT, client.read_text(&path, max_bytes)).await;
            match result {
                Ok(Ok((content, meta))) => {
                    let _ = event_tx
                        .send_async(AppEvent::SftpTextRead {
                            tab_id,
                            connection_id,
                            op_id,
                            content,
                            meta,
                        })
                        .await;
                }
                Ok(Err(error)) => {
                    let _ = event_tx
                        .send_async(AppEvent::SftpTextFailed {
                            tab_id,
                            connection_id,
                            op_id,
                            error,
                        })
                        .await;
                }
                Err(_) => {
                    let _ = event_tx
                        .send_async(AppEvent::SftpTextFailed {
                            tab_id,
                            connection_id,
                            op_id,
                            error: AppError::with_detail(
                                ErrorCode::SftpFailed,
                                format!("读取远端文本超时（{} 秒）", TEXT_READ_TIMEOUT.as_secs()),
                            ),
                        })
                        .await;
                }
            }
        }
        SftpOperation::WriteText {
            path,
            content,
            expected,
        } => {
            // 回传比读取更慢（写要落盘并等待远端确认），给更宽的超时；
            // 失败与超时统一回抛给编辑面板就地显示，不影响队列与状态栏。
            // 版本指纹冲突走独立事件：面板要弹出三选一横幅，不能按普通失败处理。
            let result = tokio::time::timeout(
                TEXT_WRITE_TIMEOUT,
                client.write_text(&path, &content, expected),
            )
            .await;
            match result {
                Ok(Ok(meta)) => {
                    let _ = event_tx
                        .send_async(AppEvent::SftpTextWritten {
                            tab_id,
                            connection_id,
                            op_id,
                            meta,
                        })
                        .await;
                }
                Ok(Err(error)) if error.code == ErrorCode::SftpTextConflict => {
                    let _ = event_tx
                        .send_async(AppEvent::SftpTextConflict {
                            tab_id,
                            connection_id,
                            op_id,
                            error,
                        })
                        .await;
                }
                Ok(Err(error)) => {
                    let _ = event_tx
                        .send_async(AppEvent::SftpTextFailed {
                            tab_id,
                            connection_id,
                            op_id,
                            error,
                        })
                        .await;
                }
                Err(_) => {
                    let _ = event_tx
                        .send_async(AppEvent::SftpTextFailed {
                            tab_id,
                            connection_id,
                            op_id,
                            error: AppError::with_detail(
                                ErrorCode::SftpFailed,
                                format!("回传远端文本超时（{} 秒）", TEXT_WRITE_TIMEOUT.as_secs()),
                            ),
                        })
                        .await;
                }
            }
        }
        SftpOperation::Symlink { target, link_path } => {
            let result = client.symlink(&target, &link_path).await;
            finish_sftp_none(event_tx, tab_id, connection_id, op_id, result).await;
        }
        SftpOperation::CompareDirs {
            local_path,
            remote_path,
            filter,
        } => match client
            .compare_directories(&local_path, &remote_path, &filter)
            .await
        {
            Ok(comparison) => {
                let _ = event_tx
                    .send_async(AppEvent::SftpDirCompared {
                        tab_id,
                        connection_id,
                        op_id,
                        comparison,
                    })
                    .await;
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
        },
        SftpOperation::SyncDirs {
            local_path,
            remote_path,
            items,
            verify,
        } => {
            let result = client
                .sync_items(
                    &local_path,
                    &remote_path,
                    &items,
                    verify,
                    throttled_progress(event_tx.clone(), tab_id, connection_id, op_id),
                )
                .await;
            finish_sftp_summary(event_tx, tab_id, connection_id, op_id, result).await;
        }
    }
}

async fn verify_transfer_result(
    client: &SftpClient,
    result: Result<u64, AppError>,
    verify: bool,
    local_path: &Path,
    remote_path: &str,
) -> Result<(u64, bool), AppError> {
    let transferred = result?;
    if verify {
        client.verify_file(local_path, remote_path).await?;
    }
    Ok((transferred, verify))
}

async fn finish_sftp(
    event_tx: flume::Sender<AppEvent>,
    tab_id: TabId,
    connection_id: ConnectionId,
    op_id: TransferId,
    result: Result<(u64, bool), AppError>,
) {
    match result {
        Ok((transferred, verified)) => {
            let _ = event_tx
                .send_async(AppEvent::SftpFinished {
                    tab_id,
                    connection_id,
                    op_id,
                    transferred: Some(transferred),
                    skipped: 0,
                    verified,
                })
                .await;
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

async fn finish_sftp_summary(
    event_tx: flume::Sender<AppEvent>,
    tab_id: TabId,
    connection_id: ConnectionId,
    op_id: TransferId,
    result: Result<transom_core::TransferSummary, AppError>,
) {
    match result {
        Ok(summary) => {
            let _ = event_tx
                .send_async(AppEvent::SftpFinished {
                    tab_id,
                    connection_id,
                    op_id,
                    transferred: Some(summary.transferred),
                    skipped: summary.skipped,
                    verified: summary.verified,
                })
                .await;
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

async fn finish_sftp_none(
    event_tx: flume::Sender<AppEvent>,
    tab_id: TabId,
    connection_id: ConnectionId,
    op_id: TransferId,
    result: Result<(), AppError>,
) {
    match result {
        Ok(()) => {
            let _ = event_tx
                .send_async(AppEvent::SftpFinished {
                    tab_id,
                    connection_id,
                    op_id,
                    transferred: None,
                    skipped: 0,
                    verified: false,
                })
                .await;
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

struct UiPrompter {
    tab_id: TabId,
    connection_id: ConnectionId,
    event_tx: flume::Sender<AppEvent>,
}

#[async_trait]
impl AuthPrompter for UiPrompter {
    /// TOFU 指纹确认：与认证提示同一条事件通道往返，握手本就阻塞在此，
    /// 等待用户作答即天然反压；弹窗被关闭时响应通道析构，按取消处理。
    async fn confirm_host_key(
        &self,
        host: &str,
        port: u16,
        key_type: &str,
        fingerprint: &str,
    ) -> Result<bool, AppError> {
        let (responder, answer) = flume::bounded(1);
        self.event_tx
            .send_async(AppEvent::HostKeyConfirm {
                tab_id: self.tab_id,
                connection_id: self.connection_id,
                host: host.to_string(),
                port,
                key_type: key_type.to_string(),
                fingerprint: fingerprint.to_string(),
                responder,
            })
            .await
            .map_err(|_| {
                AppError::with_detail(ErrorCode::Cancelled, "应用窗口已关闭，无法确认主机指纹")
            })?;
        answer
            .recv_async()
            .await
            .map_err(|_| AppError::with_detail(ErrorCode::Cancelled, "主机指纹确认已被取消"))
    }

    async fn keyboard_interactive(
        &self,
        name: &str,
        instructions: &str,
        prompts: &[PromptInfo],
    ) -> Result<Vec<String>, AppError> {
        let (responder, answers) = flume::bounded(1);
        // AuthPrompt 是用户必须看到并作答的关键事件，队列满时等待 UI 消费
        // 而不是丢弃（认证本来就阻塞在此，等待即天然反压）。
        self.event_tx
            .send_async(AppEvent::AuthPrompt {
                tab_id: self.tab_id,
                connection_id: self.connection_id,
                name: name.to_string(),
                instructions: instructions.to_string(),
                prompts: prompts.to_vec(),
                responder,
            })
            .await
            .map_err(|_| {
                AppError::with_detail(ErrorCode::Cancelled, "application window is closed")
            })?;
        answers.recv_async().await.map_err(|_| {
            AppError::with_detail(ErrorCode::Cancelled, "authentication prompt was cancelled")
        })
    }
}

fn open_local_pty(
    local: &transom_core::LocalConnection,
    size: transom_core::TerminalSize,
) -> Result<(TerminalIo, tokio::sync::oneshot::Receiver<()>), AppError> {
    use portable_pty::{CommandBuilder, PtySize, native_pty_system};
    use transom_core::{TerminalControl, TerminalControlHandle};

    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: size.rows,
            cols: size.cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| AppError::with_detail(ErrorCode::Io, format!("创建本地伪终端失败: {e}")))?;

    let shell = local.shell.clone().unwrap_or_else(|| {
        std::env::var("SHELL").unwrap_or_else(|_| {
            if cfg!(windows) {
                "powershell.exe".to_string()
            } else {
                "/bin/bash".to_string()
            }
        })
    });

    let mut command = CommandBuilder::new(&shell);
    if let Some(dir) = &local.working_dir {
        command.cwd(dir);
    }
    command.env("TERM", "xterm-256color");
    command.env("COLORTERM", "truecolor");

    let mut child = pair
        .slave
        .spawn_command(command)
        .map_err(|e| AppError::with_detail(ErrorCode::Io, format!("启动本地 Shell 失败: {e}")))?;
    drop(pair.slave);

    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| AppError::with_detail(ErrorCode::Io, e.to_string()))?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|e| AppError::with_detail(ErrorCode::Io, e.to_string()))?;

    let (control, control_rx) = TerminalControlHandle::channel(16);
    let master = pair.master;
    std::thread::Builder::new()
        .name("transom-pty-control".into())
        .spawn(move || {
            while let Ok(message) = control_rx.recv() {
                match message {
                    TerminalControl::Resize(size) => {
                        let _ = master.resize(PtySize {
                            rows: size.rows,
                            cols: size.cols,
                            pixel_width: 0,
                            pixel_height: 0,
                        });
                    }
                    TerminalControl::Close => break,
                }
            }
        })
        .map_err(|e| AppError::with_detail(ErrorCode::Io, e.to_string()))?;

    let (exit_tx, exit_rx) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("transom-pty-wait".into())
        .spawn(move || {
            let _ = child.wait();
            let _ = exit_tx.send(());
        })
        .map_err(|e| AppError::with_detail(ErrorCode::Io, e.to_string()))?;

    let io = TerminalIo::new(reader, writer, control);
    Ok((io, exit_rx))
}
