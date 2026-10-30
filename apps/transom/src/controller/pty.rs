//! UI 认证提示器与本地 PTY 打开。

use async_trait::async_trait;
use std::sync::Arc;
use transom_core::{
    AppError, AppEvent, Connection, ConnectionId, ConnectionSecrets, ErrorCode, KnownHostsStore,
    LocalConnection, MoshConnection, OutboundProxy, PromptInfo, SerialConnection, Session,
    SshConnection, SshHop, TabId, TelnetConnection, TerminalAppEvent, TerminalIo, TerminalSize,
};
use transom_net::{
    AuthPrompter, SshConnectParams, build_mosh_server_command, check_local_mosh_client,
    connect_ssh, connect_telnet, open_serial, parse_mosh_server_output,
};
use transom_zmodem::ZmodemConfig;

use super::{Internal, ReadyConnection, local_shell};

pub(super) struct UiPrompter {
    pub(super) tab_id: TabId,
    pub(super) connection_id: ConnectionId,
    pub(super) event_tx: flume::Sender<AppEvent>,
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
        let (responder, mut answer) = tokio::sync::mpsc::channel(1);
        self.event_tx
            .send_async(
                TerminalAppEvent::HostKeyConfirm {
                    tab_id: self.tab_id,
                    connection_id: self.connection_id,
                    host: host.to_string(),
                    port,
                    key_type: key_type.to_string(),
                    fingerprint: fingerprint.to_string(),
                    responder,
                }
                .into(),
            )
            .await
            .map_err(|_| {
                AppError::with_detail(ErrorCode::Cancelled, "应用窗口已关闭，无法确认主机指纹")
            })?;
        answer
            .recv()
            .await
            .ok_or_else(|| AppError::with_detail(ErrorCode::Cancelled, "主机指纹确认已被取消"))
    }

    async fn keyboard_interactive(
        &self,
        name: &str,
        instructions: &str,
        prompts: &[PromptInfo],
    ) -> Result<Vec<String>, AppError> {
        let (responder, mut answers) = tokio::sync::mpsc::channel(1);
        // AuthPrompt 是用户必须看到并作答的关键事件，队列满时等待 UI 消费
        // 而不是丢弃（认证本来就阻塞在此，等待即天然反压）。
        self.event_tx
            .send_async(
                TerminalAppEvent::AuthPrompt {
                    tab_id: self.tab_id,
                    connection_id: self.connection_id,
                    name: name.to_string(),
                    instructions: instructions.to_string(),
                    prompts: prompts.to_vec(),
                    responder,
                }
                .into(),
            )
            .await
            .map_err(|_| AppError::with_detail(ErrorCode::Cancelled, "应用窗口已关闭"))?;
        answers
            .recv()
            .await
            .ok_or_else(|| AppError::with_detail(ErrorCode::Cancelled, "认证提示已被取消"))
    }
}

pub(super) fn open_local_pty(
    local: &LocalConnection,
    size: TerminalSize,
    shell_integration: bool,
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

    // `X-TerminalArgExec`：按 argv 原样执行，不经 Shell 拼串。空 argv 视为
    // 未指定（防御脏配置），回落到交互式 Shell。
    let exec_argv = local
        .command
        .as_ref()
        .filter(|argv| !argv.is_empty())
        .cloned();
    let mut command = match &exec_argv {
        Some(argv) => {
            let mut builder = CommandBuilder::new(&argv[0]);
            builder.args(&argv[1..]);
            builder
        }
        None => CommandBuilder::new(&shell),
    };
    if let Some(dir) = &local.working_dir {
        command.cwd(dir);
    }
    command.env("TERM", "xterm-256color");
    command.env("COLORTERM", "truecolor");
    // 集成预装：能在启动参数里把钩子装好，就不必往终端写任何字节，
    // 屏幕上不会出现注入痕迹，也没有一闪而过。命令执行模式下不预装：
    // 目标程序不是交互式 Shell，注入的 rcfile 参数会落到它头上。
    let preload = if shell_integration && exec_argv.is_none() {
        local_shell::prepare(&shell)
    } else {
        local_shell::LocalShellPreload::none()
    };
    command.args(&preload.args);
    for (key, value) in &preload.env {
        command.env(key, value);
    }

    let mut child = pair
        .slave
        .spawn_command(command)
        .map_err(|e| AppError::with_detail(ErrorCode::Io, format!("启动本地 Shell 失败: {e}")))?;
    drop(pair.slave);
    log::info!(
        "本地伪终端已就绪，Shell: {shell}，尺寸: {}x{}，工作目录: {:?}，执行命令: {exec_argv:?}",
        size.cols,
        size.rows,
        local.working_dir
    );

    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| AppError::with_detail(ErrorCode::Io, e.to_string()))?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|e| AppError::with_detail(ErrorCode::Io, e.to_string()))?;

    let (control, mut control_rx) = TerminalControlHandle::channel(16);
    let master = pair.master;
    std::thread::Builder::new()
        .name("transom-pty-control".into())
        .spawn(move || {
            while let Some(message) = control_rx.blocking_recv() {
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
            // 退出原因必须留痕：界面只看得见"标签自己关了"，到底是被 exit 掉的
            // 还是被信号打死的，只有这里有答案。
            match child.wait() {
                Ok(status) => log::info!("本地 Shell 进程已退出，状态: {status:?}"),
                Err(error) => log::warn!("等待本地 Shell 进程退出失败，原因: {error}"),
            }
            let _ = exit_tx.send(());
        })
        .map_err(|e| AppError::with_detail(ErrorCode::Io, e.to_string()))?;

    let mut io = TerminalIo::new(reader, writer, control);
    if let Some(kind) = preload.kind {
        io = io.with_preinstalled_integration(kind);
        log::info!("本地 Shell 集成已通过启动参数预装，方言: {}", kind.label());
    }
    if let Some(cleanup) = preload.cleanup {
        // 清理守卫随会话句柄释放：临时 rcfile 目录必须活到 Shell 读完启动文件，
        // 又不能留在磁盘上等系统回收。
        io.cleanup = cleanup;
    }
    Ok((io, exit_rx))
}

/// 连接派发参数：主循环把 Connect 命令的全部载荷收敛到这里。
pub(super) struct ConnectDispatch {
    pub(super) known_hosts: Arc<dyn KnownHostsStore>,
    pub(super) event_tx: flume::Sender<AppEvent>,
    pub(super) internal_tx: flume::Sender<Internal>,
    pub(super) tab_id: TabId,
    pub(super) connection_id: ConnectionId,
    pub(super) session: Box<Session>,
    /// 已解析的跳板链（最外层到最内层），SSH 会话按序建链。
    pub(super) jumps: Vec<SshHop>,
    /// 实际生效的出站代理。
    pub(super) proxy: OutboundProxy,
    pub(super) size: TerminalSize,
    pub(super) secrets: ConnectionSecrets,
    /// ZMODEM 监督层配置（随命令携带，控制器不回读设置文件）。
    pub(super) zmodem: ZmodemConfig,
    /// 是否允许 SHA1（`ssh-rsa`）用户公钥认证（随命令携带）。
    pub(super) allow_sha1_publickey_auth: bool,
    /// 是否启用 Shell 集成（随命令携带）；本地会话据此决定要不要用启动参数预装。
    pub(super) shell_integration: bool,
}

/// 按连接类型派发建连：SSH 走认证握手，本地开 PTY，串口走阻塞线程池打开。
/// 结果以 Internal::*Connected 回流主循环，失败直接广播 ConnectionFailed。
/// 返回任务句柄供调用方登记意图、被取代时取消。
pub(super) fn dispatch_connect(dispatch: ConnectDispatch) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let outcome = match &dispatch.session.connection {
            Connection::Ssh(ssh) => connect_ssh_task(&dispatch, ssh).await,
            Connection::Mosh(mosh) => connect_mosh_task(&dispatch, mosh).await,
            Connection::Local(local) => {
                connect_local_task(local.clone(), dispatch.size, dispatch.shell_integration).await
            }
            Connection::Serial(serial) => connect_serial_task(serial.clone()).await,
            Connection::Telnet(telnet) => connect_telnet_task(telnet.clone(), dispatch.size).await,
            _ => Err(AppError::with_detail(
                ErrorCode::Configuration,
                "不支持的连接类型",
            )),
        };

        match outcome {
            Ok(kind) => {
                let _ = dispatch
                    .internal_tx
                    .send_async(Internal::Connected {
                        tab_id: dispatch.tab_id,
                        connection_id: dispatch.connection_id,
                        kind,
                        zmodem: dispatch.zmodem,
                    })
                    .await;
            }
            Err(error) => {
                let _ = dispatch
                    .event_tx
                    .send_async(
                        TerminalAppEvent::ConnectionFailed {
                            tab_id: dispatch.tab_id,
                            connection_id: dispatch.connection_id,
                            error,
                        }
                        .into(),
                    )
                    .await;
            }
        }
    })
}

async fn connect_ssh_task(
    dispatch: &ConnectDispatch,
    ssh: &SshConnection,
) -> Result<ReadyConnection, AppError> {
    let prompter = Arc::new(UiPrompter {
        tab_id: dispatch.tab_id,
        connection_id: dispatch.connection_id,
        event_tx: dispatch.event_tx.clone(),
    });
    let params = SshConnectParams {
        connection: ssh,
        jumps: &dispatch.jumps,
        size: dispatch.size,
        credentials: dispatch.secrets.clone(),
        prompter,
        known_hosts: dispatch.known_hosts.clone(),
        proxy: dispatch.proxy.clone(),
        allow_sha1_publickey_auth: dispatch.allow_sha1_publickey_auth,
    };
    let session = connect_ssh(params).await?;
    Ok(ReadyConnection::Ssh(session))
}

async fn connect_local_task(
    local: LocalConnection,
    size: TerminalSize,
    shell_integration: bool,
) -> Result<ReadyConnection, AppError> {
    // PTY 创建与 Shell 启动是阻塞系统调用，禁止占用异步工作线程。
    tokio::task::spawn_blocking(move || open_local_pty(&local, size, shell_integration))
        .await
        .map_err(|error| {
            AppError::with_detail(
                ErrorCode::Io,
                format!("本地伪终端创建任务异常终止: {error}"),
            )
        })?
        .map(|(io, join)| ReadyConnection::Local { io, join })
}

async fn connect_serial_task(serial: SerialConnection) -> Result<ReadyConnection, AppError> {
    // 串口打开是阻塞系统调用，禁止占用异步工作线程。
    tokio::task::spawn_blocking(move || open_serial(&serial))
        .await
        .map_err(|error| {
            AppError::with_detail(ErrorCode::Io, format!("串口连接任务异常终止: {error}"))
        })?
        .map(|connected| ReadyConnection::Serial {
            io: connected.io,
            join: connected.exit,
        })
}

async fn connect_telnet_task(
    telnet: TelnetConnection,
    size: TerminalSize,
) -> Result<ReadyConnection, AppError> {
    // TCP 拨号与域名解析是阻塞系统调用，禁止占用异步工作线程。
    tokio::task::spawn_blocking(move || connect_telnet(&telnet, size))
        .await
        .map_err(|error| {
            AppError::with_detail(ErrorCode::Io, format!("Telnet 连接任务异常终止: {error}"))
        })?
        .map(|connected| ReadyConnection::Telnet {
            io: connected.io,
            join: connected.exit,
        })
}

async fn connect_mosh_task(
    dispatch: &ConnectDispatch,
    mosh: &MoshConnection,
) -> Result<ReadyConnection, AppError> {
    use portable_pty::{CommandBuilder, PtySize, native_pty_system};
    use transom_core::TerminalControlHandle;

    // 1. 引导阶段：建立 SSH 连接并在远端执行 mosh-server
    let ssh_equiv = SshConnection {
        host: mosh.host.clone(),
        port: mosh.ssh_port,
        username: mosh.username.clone(),
        auth: mosh.auth.clone(),
        options: mosh.options.clone(),
        proxy_jump: None,
    };
    let prompter = Arc::new(UiPrompter {
        tab_id: dispatch.tab_id,
        connection_id: dispatch.connection_id,
        event_tx: dispatch.event_tx.clone(),
    });
    let params = SshConnectParams {
        connection: &ssh_equiv,
        jumps: &dispatch.jumps,
        size: dispatch.size,
        credentials: dispatch.secrets.clone(),
        prompter,
        known_hosts: dispatch.known_hosts.clone(),
        proxy: dispatch.proxy.clone(),
        allow_sha1_publickey_auth: dispatch.allow_sha1_publickey_auth,
    };
    let ssh_session = connect_ssh(params).await?;
    let cmd = build_mosh_server_command(&mosh.server_path, mosh.server_port);
    log::info!("正在通过 SSH 引导远端 mosh-server，执行命令: {cmd}");
    let output = ssh_session.remote.exec_command_capture(&cmd).await?;
    let info = parse_mosh_server_output(&output.stdout)?;
    log::info!("远端 mosh-server 启动成功，UDP 监听端口: {}", info.udp_port);

    // 2. 本地客户端检查
    if !check_local_mosh_client() {
        return Err(AppError::with_detail(
            ErrorCode::Configuration,
            format!(
                "远端 mosh-server 已成功启动（UDP 端口: {}），但本地未检测到 mosh-client。请先通过系统包管理器安装 mosh（例如 apt/brew/dnf install mosh）",
                info.udp_port
            ),
        ));
    }

    // 3. 本地启动 mosh-client 桥接伪终端
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: dispatch.size.rows,
            cols: dispatch.size.cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| {
            AppError::with_detail(ErrorCode::Io, format!("创建 Mosh 本地伪终端失败: {e}"))
        })?;

    let mut command = CommandBuilder::new("mosh-client");
    command.env("MOSH_KEY", &info.session_key);
    command.env("TERM", "xterm-256color");
    command.env("COLORTERM", "truecolor");
    command.arg(&mosh.host);
    command.arg(info.udp_port.to_string());

    let mut child = pair.slave.spawn_command(command).map_err(|e| {
        AppError::with_detail(ErrorCode::Io, format!("启动本地 mosh-client 失败: {e}"))
    })?;
    drop(pair.slave);

    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| AppError::with_detail(ErrorCode::Io, e.to_string()))?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|e| AppError::with_detail(ErrorCode::Io, e.to_string()))?;

    let (control, mut control_rx) = TerminalControlHandle::channel(16);
    let master = pair.master;
    std::thread::Builder::new()
        .name("transom-mosh-pty-control".into())
        .spawn(move || {
            while let Some(message) = control_rx.blocking_recv() {
                match message {
                    transom_core::TerminalControl::Resize(size) => {
                        let _ = master.resize(PtySize {
                            rows: size.rows,
                            cols: size.cols,
                            pixel_width: 0,
                            pixel_height: 0,
                        });
                    }
                    transom_core::TerminalControl::Close => break,
                }
            }
        })
        .map_err(|e| AppError::with_detail(ErrorCode::Io, e.to_string()))?;

    let (exit_tx, exit_rx) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("transom-mosh-exit".into())
        .spawn(move || {
            let _ = child.wait();
            let _ = exit_tx.send(());
        })
        .map_err(|e| AppError::with_detail(ErrorCode::Io, e.to_string()))?;

    let io = TerminalIo::new(reader, writer, control);
    Ok(ReadyConnection::Local { io, join: exit_rx })
}
