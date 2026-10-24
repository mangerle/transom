use async_trait::async_trait;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tokio::process::Command;
use tokio::runtime::Runtime;
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
        let (command_tx, command_rx) = flume::unbounded();
        let (event_tx, event_rx) = flume::unbounded();
        let (internal_tx, internal_rx) = flume::unbounded();
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
                    .send(AppEvent::NetworkTrafficUpdated {
                        tab_id,
                        connection_id,
                        traffic: sample,
                    })
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
                                    let _ = internal_tx.send(Internal::Connected {
                                        tab_id,
                                        connection_id,
                                        session,
                                    });
                                }
                                Err(error) => {
                                    let _ = event_tx.send(AppEvent::ConnectionFailed {
                                        tab_id,
                                        connection_id,
                                        error,
                                    });
                                }
                            }
                        }
                        Connection::Local(local) => match open_local_pty(local, size) {
                            Ok((io, join)) => {
                                let _ = internal_tx.send(Internal::LocalConnected {
                                    tab_id,
                                    connection_id,
                                    io,
                                    join,
                                });
                            }
                            Err(error) => {
                                let _ = event_tx.send(AppEvent::ConnectionFailed {
                                    tab_id,
                                    connection_id,
                                    error,
                                });
                            }
                        },
                        Connection::Serial(serial) => {
                            let serial = serial.clone();
                            let internal_tx = internal_tx.clone();
                            let event_tx = event_tx.clone();
                            // 串口打开是阻塞系统调用，禁止占用异步工作线程。
                            match tokio::task::spawn_blocking(move || open_serial(&serial)).await {
                                Ok(Ok(connected)) => {
                                    let _ = internal_tx.send(Internal::SerialConnected {
                                        tab_id,
                                        connection_id,
                                        io: connected.io,
                                        join: connected.exit,
                                    });
                                }
                                Ok(Err(error)) => {
                                    let _ = event_tx.send(AppEvent::ConnectionFailed {
                                        tab_id,
                                        connection_id,
                                        error,
                                    });
                                }
                                Err(error) => {
                                    let _ = event_tx.send(AppEvent::ConnectionFailed {
                                        tab_id,
                                        connection_id,
                                        error: AppError::with_detail(
                                            ErrorCode::Io,
                                            format!("串口连接任务异常终止: {error}"),
                                        ),
                                    });
                                }
                            }
                        }
                        _ => {
                            let _ = event_tx.send(AppEvent::ConnectionFailed {
                                tab_id,
                                connection_id,
                                error: AppError::with_detail(
                                    ErrorCode::Configuration,
                                    "不支持的连接类型",
                                ),
                            });
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
                    let _ = event_tx.send(AppEvent::SftpFailed {
                        tab_id,
                        connection_id,
                        op_id,
                        error: AppError::with_detail(
                            ErrorCode::ChannelFailed,
                            "SSH connection is not ready",
                        ),
                    });
                    continue;
                };
                if current_connection != connection_id {
                    let _ = event_tx.send(AppEvent::SftpFailed {
                        tab_id,
                        connection_id,
                        op_id,
                        error: AppError::with_detail(
                            ErrorCode::ChannelFailed,
                            "SSH connection changed",
                        ),
                    });
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
                            let _ = event_tx.send(AppEvent::SftpFailed {
                                tab_id,
                                connection_id,
                                op_id,
                                error,
                            });
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
                let handle = tokio::spawn(async move {
                    execute_sftp_op(event_tx, tab_id, connection_id, op_id, client, op).await;
                });
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
                    let _ = event_tx.send(AppEvent::ForwardChanged {
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
                    });
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
                let _ = event_tx.send(AppEvent::ForwardChanged {
                    tab_id,
                    connection_id,
                    status,
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
                let _ = event_tx.send(AppEvent::ForwardChanged {
                    tab_id,
                    connection_id,
                    status: ForwardStatus {
                        rule_id,
                        state: ForwardState::Stopped,
                        error: None,
                    },
                });
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
                            let _ = event_tx.send(AppEvent::PublicKeyDeployed {
                                tab_id,
                                connection_id,
                                key_name,
                                error,
                            });
                        });
                        key_deploy_tasks.insert(tab_id, handle);
                    }
                    None => {
                        let _ = event_tx.send(AppEvent::PublicKeyDeployed {
                            tab_id,
                            connection_id,
                            key_name,
                            error: Some(AppError::with_detail(
                                ErrorCode::ChannelFailed,
                                "SSH 连接尚未就绪，无法部署公钥",
                            )),
                        });
                    }
                }
            }
            Incoming::Command(Ok(AppCommand::CloseTab { tab_id })) => {
                current_connections.remove(&tab_id);
                stop_forwards(&mut forwards, tab_id);
                stop_traffic(&mut traffic_tasks, tab_id);
                stop_key_deploy(&mut key_deploy_tasks, tab_id);
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
                let _ = event_tx.send(AppEvent::TerminalReady {
                    tab_id,
                    connection_id,
                    io,
                });
                let internal_tx = internal_tx.clone();
                tokio::spawn(async move {
                    let _ = join.await;
                    let _ = internal_tx.send(Internal::Disconnected {
                        tab_id,
                        connection_id,
                    });
                });

                let traffic_task =
                    spawn_traffic_sampler(traffic, tab_id, connection_id, event_tx.clone());
                traffic_tasks.insert(tab_id, traffic_task);

                let event_tx_stats = event_tx.clone();
                tokio::spawn(async move {
                    let mut prev_cpu = None;
                    let mut interval = tokio::time::interval(std::time::Duration::from_secs(3));
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    loop {
                        let cmd = "cat /proc/stat /proc/meminfo 2>/dev/null; echo \"__UPTIME__\"; cat /proc/uptime 2>/dev/null; echo \"__DF__\"; df -P -k / 2>/dev/null; echo \"__OS__\"; cat /etc/os-release 2>/dev/null";
                        match remote_clone.exec_command(cmd).await {
                            Ok(output) if !output.is_empty() => {
                                let (stats, curr_cpu) =
                                    transom_core::system_stats::parse_combined_stats(
                                        &output, prev_cpu,
                                    );
                                prev_cpu = curr_cpu;
                                if event_tx_stats
                                    .send(AppEvent::SystemStatsUpdated {
                                        tab_id,
                                        connection_id,
                                        stats,
                                    })
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
                let _ = event_tx.send(AppEvent::TerminalReady {
                    tab_id,
                    connection_id,
                    io,
                });
                let internal_tx = internal_tx.clone();
                tokio::spawn(async move {
                    let _ = join.await;
                    let _ = internal_tx.send(Internal::Disconnected {
                        tab_id,
                        connection_id,
                    });
                });

                let event_tx_stats = event_tx.clone();
                tokio::spawn(async move {
                    let mut prev_cpu = None;
                    let mut interval = tokio::time::interval(std::time::Duration::from_secs(3));
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    loop {
                        let stat = tokio::fs::read_to_string("/proc/stat")
                            .await
                            .unwrap_or_default();
                        let mem = tokio::fs::read_to_string("/proc/meminfo")
                            .await
                            .unwrap_or_default();
                        let uptime = tokio::fs::read_to_string("/proc/uptime")
                            .await
                            .unwrap_or_default();
                        let os = tokio::fs::read_to_string("/etc/os-release")
                            .await
                            .unwrap_or_default();
                        let df = match Command::new("df").args(["-P", "-k", "/"]).output().await {
                            Ok(out) => String::from_utf8_lossy(&out.stdout).to_string(),
                            Err(_) => String::new(),
                        };
                        let output = format!(
                            "{stat}\n{mem}\n__UPTIME__\n{uptime}\n__DF__\n{df}\n__OS__\n{os}"
                        );
                        let (stats, curr_cpu) =
                            transom_core::system_stats::parse_combined_stats(&output, prev_cpu);
                        prev_cpu = curr_cpu;
                        if event_tx_stats
                            .send(AppEvent::SystemStatsUpdated {
                                tab_id,
                                connection_id,
                                stats,
                            })
                            .is_err()
                        {
                            break;
                        }
                        interval.tick().await;
                    }
                });
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
                let _ = event_tx.send(AppEvent::TerminalReady {
                    tab_id,
                    connection_id,
                    io,
                });
                let internal_tx = internal_tx.clone();
                tokio::spawn(async move {
                    let _ = join.await;
                    let _ = internal_tx.send(Internal::Disconnected {
                        tab_id,
                        connection_id,
                    });
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
                    let _ = event_tx.send(AppEvent::Disconnected {
                        tab_id,
                        connection_id,
                    });
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
                let _ = event_tx.send(AppEvent::SftpListing {
                    tab_id,
                    connection_id,
                    path,
                    entries,
                });
            }
            Err(error) => {
                let _ = event_tx.send(AppEvent::SftpFailed {
                    tab_id,
                    connection_id,
                    op_id,
                    error,
                });
            }
        },
        SftpOperation::Download {
            remote_path,
            local_path,
            resume,
            verify,
        } => {
            let progress_tx = event_tx.clone();
            let result = client
                .download_resumable(
                    &remote_path,
                    &local_path,
                    resume,
                    move |transferred, total| {
                        let _ = progress_tx.send(AppEvent::SftpProgress {
                            tab_id,
                            connection_id,
                            op_id,
                            transferred,
                            total,
                        });
                    },
                )
                .await;
            let result =
                verify_transfer_result(&client, result, verify, &local_path, &remote_path).await;
            finish_sftp(event_tx, tab_id, connection_id, op_id, result);
        }
        SftpOperation::Upload {
            local_path,
            remote_path,
            resume,
            verify,
        } => {
            let progress_tx = event_tx.clone();
            let result = client
                .upload_resumable(
                    &local_path,
                    &remote_path,
                    resume,
                    move |transferred, total| {
                        let _ = progress_tx.send(AppEvent::SftpProgress {
                            tab_id,
                            connection_id,
                            op_id,
                            transferred,
                            total,
                        });
                    },
                )
                .await;
            let result =
                verify_transfer_result(&client, result, verify, &local_path, &remote_path).await;
            finish_sftp(event_tx, tab_id, connection_id, op_id, result);
        }
        SftpOperation::UploadDir {
            local_path,
            remote_path,
            resume,
            filter,
            verify,
        } => {
            let progress_tx = event_tx.clone();
            let result = client
                .upload_dir(
                    &local_path,
                    &remote_path,
                    resume,
                    &filter,
                    verify,
                    move |transferred, total| {
                        let _ = progress_tx.send(AppEvent::SftpProgress {
                            tab_id,
                            connection_id,
                            op_id,
                            transferred,
                            total,
                        });
                    },
                )
                .await;
            finish_sftp_summary(event_tx, tab_id, connection_id, op_id, result);
        }
        SftpOperation::DownloadDir {
            remote_path,
            local_path,
            resume,
            filter,
            verify,
        } => {
            let progress_tx = event_tx.clone();
            let result = client
                .download_dir(
                    &remote_path,
                    &local_path,
                    resume,
                    &filter,
                    verify,
                    move |transferred, total| {
                        let _ = progress_tx.send(AppEvent::SftpProgress {
                            tab_id,
                            connection_id,
                            op_id,
                            transferred,
                            total,
                        });
                    },
                )
                .await;
            finish_sftp_summary(event_tx, tab_id, connection_id, op_id, result);
        }
        SftpOperation::Mkdir { path } => {
            let result = client.create_dir(&path).await;
            finish_sftp_none(event_tx, tab_id, connection_id, op_id, result);
        }
        SftpOperation::Rename { from, to } => {
            let result = client.rename(&from, &to).await;
            finish_sftp_none(event_tx, tab_id, connection_id, op_id, result);
        }
        SftpOperation::Delete { path, is_dir } => {
            let result = client.delete(&path, is_dir).await;
            finish_sftp_none(event_tx, tab_id, connection_id, op_id, result);
        }
        SftpOperation::DeleteMany { targets } => {
            let mut result = Ok(());
            for target in targets {
                if let Err(error) = client.delete(&target.path, target.is_dir).await {
                    result = Err(error);
                    break;
                }
            }
            finish_sftp_none(event_tx, tab_id, connection_id, op_id, result);
        }
        SftpOperation::Chmod { path, permissions } => {
            let result = client.chmod(&path, permissions).await;
            finish_sftp_none(event_tx, tab_id, connection_id, op_id, result);
        }
        SftpOperation::Stat { path } => match client.stat(&path).await {
            Ok(metadata) => {
                let _ = event_tx.send(AppEvent::SftpMetadata {
                    tab_id,
                    connection_id,
                    op_id,
                    metadata,
                });
            }
            Err(error) => {
                let _ = event_tx.send(AppEvent::SftpFailed {
                    tab_id,
                    connection_id,
                    op_id,
                    error,
                });
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
                    let _ = event_tx.send(AppEvent::SftpPreview {
                        tab_id,
                        connection_id,
                        op_id,
                        bytes,
                        truncated,
                    });
                }
                Ok(Err(error)) => {
                    let _ = event_tx.send(AppEvent::SftpPreviewFailed {
                        tab_id,
                        connection_id,
                        op_id,
                        error,
                    });
                }
                Err(_) => {
                    let _ = event_tx.send(AppEvent::SftpPreviewFailed {
                        tab_id,
                        connection_id,
                        op_id,
                        error: AppError::with_detail(
                            ErrorCode::SftpFailed,
                            "读取远端预览超时（10 秒）",
                        ),
                    });
                }
            }
        }
        SftpOperation::Symlink { target, link_path } => {
            let result = client.symlink(&target, &link_path).await;
            finish_sftp_none(event_tx, tab_id, connection_id, op_id, result);
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
                let _ = event_tx.send(AppEvent::SftpDirCompared {
                    tab_id,
                    connection_id,
                    op_id,
                    comparison,
                });
            }
            Err(error) => {
                let _ = event_tx.send(AppEvent::SftpFailed {
                    tab_id,
                    connection_id,
                    op_id,
                    error,
                });
            }
        },
        SftpOperation::SyncDirs {
            local_path,
            remote_path,
            items,
            verify,
        } => {
            let progress_tx = event_tx.clone();
            let result = client
                .sync_items(
                    &local_path,
                    &remote_path,
                    &items,
                    verify,
                    move |transferred, total| {
                        let _ = progress_tx.send(AppEvent::SftpProgress {
                            tab_id,
                            connection_id,
                            op_id,
                            transferred,
                            total,
                        });
                    },
                )
                .await;
            finish_sftp_summary(event_tx, tab_id, connection_id, op_id, result);
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

fn finish_sftp(
    event_tx: flume::Sender<AppEvent>,
    tab_id: TabId,
    connection_id: ConnectionId,
    op_id: TransferId,
    result: Result<(u64, bool), AppError>,
) {
    match result {
        Ok((transferred, verified)) => {
            let _ = event_tx.send(AppEvent::SftpFinished {
                tab_id,
                connection_id,
                op_id,
                transferred: Some(transferred),
                skipped: 0,
                verified,
            });
        }
        Err(error) => {
            let _ = event_tx.send(AppEvent::SftpFailed {
                tab_id,
                connection_id,
                op_id,
                error,
            });
        }
    }
}

fn finish_sftp_summary(
    event_tx: flume::Sender<AppEvent>,
    tab_id: TabId,
    connection_id: ConnectionId,
    op_id: TransferId,
    result: Result<transom_core::TransferSummary, AppError>,
) {
    match result {
        Ok(summary) => {
            let _ = event_tx.send(AppEvent::SftpFinished {
                tab_id,
                connection_id,
                op_id,
                transferred: Some(summary.transferred),
                skipped: summary.skipped,
                verified: summary.verified,
            });
        }
        Err(error) => {
            let _ = event_tx.send(AppEvent::SftpFailed {
                tab_id,
                connection_id,
                op_id,
                error,
            });
        }
    }
}

fn finish_sftp_none(
    event_tx: flume::Sender<AppEvent>,
    tab_id: TabId,
    connection_id: ConnectionId,
    op_id: TransferId,
    result: Result<(), AppError>,
) {
    match result {
        Ok(()) => {
            let _ = event_tx.send(AppEvent::SftpFinished {
                tab_id,
                connection_id,
                op_id,
                transferred: None,
                skipped: 0,
                verified: false,
            });
        }
        Err(error) => {
            let _ = event_tx.send(AppEvent::SftpFailed {
                tab_id,
                connection_id,
                op_id,
                error,
            });
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
    async fn keyboard_interactive(
        &self,
        name: &str,
        instructions: &str,
        prompts: &[PromptInfo],
    ) -> Result<Vec<String>, AppError> {
        let (responder, answers) = flume::bounded(1);
        self.event_tx
            .send(AppEvent::AuthPrompt {
                tab_id: self.tab_id,
                connection_id: self.connection_id,
                name: name.to_string(),
                instructions: instructions.to_string(),
                prompts: prompts.to_vec(),
                responder,
            })
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
