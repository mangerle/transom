//! UI 认证提示器与本地 PTY 打开。

use async_trait::async_trait;
use std::sync::Arc;
use transom_core::{
    AppError, AppEvent, Connection, ConnectionId, ConnectionSecrets, ErrorCode, KnownHostsStore,
    PromptInfo, Session, TabId, TerminalIo, TerminalSize,
};
use transom_net::{AuthPrompter, connect_ssh, connect_telnet, open_serial};

use super::{Internal, ReadyConnection};

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
            .send_async(AppEvent::AuthPrompt {
                tab_id: self.tab_id,
                connection_id: self.connection_id,
                name: name.to_string(),
                instructions: instructions.to_string(),
                prompts: prompts.to_vec(),
                responder,
            })
            .await
            .map_err(|_| AppError::with_detail(ErrorCode::Cancelled, "应用窗口已关闭"))?;
        answers
            .recv()
            .await
            .ok_or_else(|| AppError::with_detail(ErrorCode::Cancelled, "认证提示已被取消"))
    }
}

pub(super) fn open_local_pty(
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
    log::info!(
        "本地伪终端已就绪，Shell: {shell}，尺寸: {}x{}，工作目录: {:?}",
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

    let io = TerminalIo::new(reader, writer, control);
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
    pub(super) jumps: Vec<transom_core::SshHop>,
    /// 实际生效的出站代理。
    pub(super) proxy: transom_core::OutboundProxy,
    pub(super) size: TerminalSize,
    pub(super) secrets: ConnectionSecrets,
}

/// 按连接类型派发建连：SSH 走认证握手，本地开 PTY，串口走阻塞线程池打开。
/// 结果以 Internal::*Connected 回流主循环，失败直接广播 ConnectionFailed。
/// 返回任务句柄供调用方登记意图、被取代时取消。
pub(super) fn dispatch_connect(dispatch: ConnectDispatch) -> tokio::task::JoinHandle<()> {
    let ConnectDispatch {
        known_hosts,
        event_tx,
        internal_tx,
        tab_id,
        connection_id,
        session,
        jumps,
        proxy,
        size,
        secrets,
    } = dispatch;
    tokio::spawn(async move {
        match &session.connection {
            Connection::Ssh(ssh) => {
                let prompter = Arc::new(UiPrompter {
                    tab_id,
                    connection_id,
                    event_tx: event_tx.clone(),
                });
                let params = transom_net::SshConnectParams {
                    connection: ssh,
                    jumps: &jumps,
                    size,
                    credentials: secrets,
                    prompter,
                    known_hosts,
                    proxy,
                };
                match connect_ssh(params).await {
                    Ok(session) => {
                        let _ = internal_tx
                            .send_async(Internal::Connected {
                                tab_id,
                                connection_id,
                                kind: ReadyConnection::Ssh(session),
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
            Connection::Local(local) => {
                let local = local.clone();
                // PTY 创建与 Shell 启动是阻塞系统调用（与串口打开同口径），
                // 禁止占用异步工作线程：多标签同时启动时会挤占网络任务调度。
                match tokio::task::spawn_blocking(move || open_local_pty(&local, size)).await {
                    Ok(Ok((io, join))) => {
                        let _ = internal_tx
                            .send_async(Internal::Connected {
                                tab_id,
                                connection_id,
                                kind: ReadyConnection::Local { io, join },
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
                                    format!("本地伪终端创建任务异常终止: {error}"),
                                ),
                            })
                            .await;
                    }
                }
            }
            Connection::Serial(serial) => {
                let serial = serial.clone();
                let internal_tx = internal_tx.clone();
                let event_tx = event_tx.clone();
                // 串口打开是阻塞系统调用，禁止占用异步工作线程。
                match tokio::task::spawn_blocking(move || open_serial(&serial)).await {
                    Ok(Ok(connected)) => {
                        let _ = internal_tx
                            .send_async(Internal::Connected {
                                tab_id,
                                connection_id,
                                kind: ReadyConnection::Serial {
                                    io: connected.io,
                                    join: connected.exit,
                                },
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
            Connection::Telnet(telnet) => {
                let telnet = telnet.clone();
                let internal_tx = internal_tx.clone();
                let event_tx = event_tx.clone();
                // TCP 拨号与域名解析是阻塞系统调用，禁止占用异步工作线程。
                match tokio::task::spawn_blocking(move || connect_telnet(&telnet, size)).await {
                    Ok(Ok(connected)) => {
                        let _ = internal_tx
                            .send_async(Internal::Connected {
                                tab_id,
                                connection_id,
                                kind: ReadyConnection::Telnet {
                                    io: connected.io,
                                    join: connected.exit,
                                },
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
                                    format!("Telnet 连接任务异常终止: {error}"),
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
                        error: AppError::with_detail(ErrorCode::Configuration, "不支持的连接类型"),
                    })
                    .await;
            }
        }
    })
}
