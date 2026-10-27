//! SSH 会话句柄、远端命令执行与伪终端协程。

use super::handler::{ClientHandler, RemoteForwardTasks};
use russh::client::{Handle, Msg};
use russh::{Channel, ChannelMsg, Disconnect};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::task::JoinHandle;
use transom_core::{AppError, ErrorCode, TerminalControl, TerminalIo, TrafficCounter};

/// 单次远端命令允许捕获的输出总量上限（stdout 与 stderr 合计）。
pub(crate) const MAX_EXEC_OUTPUT_BYTES: usize = 4 * 1024 * 1024;

/// 单次远端命令的兜底超时。
pub(crate) const EXEC_COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

/// 错误信息中命令文本的最大字符数（截断点回退到字符边界）。
pub(crate) const COMMAND_EXCERPT_BYTES: usize = 120;

/// 远端命令执行结果；`exit_status` 缺失表示服务端未回传退出码。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    pub exit_status: Option<u32>,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Clone)]
pub struct SshRemote {
    pub(crate) inner: Arc<Handle<ClientHandler>>,
    pub(crate) remote_forwards: Arc<Mutex<HashMap<u16, (String, u16)>>>,
    pub(crate) forward_tasks: RemoteForwardTasks,
    pub(crate) jump_handles: Vec<Arc<Handle<ClientHandler>>>,
}

impl SshRemote {
    pub(crate) fn new(
        inner: Arc<Handle<ClientHandler>>,
        remote_forwards: Arc<Mutex<HashMap<u16, (String, u16)>>>,
        forward_tasks: RemoteForwardTasks,
        jump_handles: Vec<Arc<Handle<ClientHandler>>>,
    ) -> Self {
        Self {
            inner,
            remote_forwards,
            forward_tasks,
            jump_handles,
        }
    }

    pub(crate) async fn open_session(&self) -> Result<Channel<Msg>, AppError> {
        self.inner
            .channel_open_session()
            .await
            .map_err(|error| AppError::with_detail(ErrorCode::ChannelFailed, error.to_string()))
    }

    pub(crate) async fn open_direct_tcpip(
        &self,
        host: &str,
        port: u16,
    ) -> Result<Channel<Msg>, AppError> {
        self.inner
            .channel_open_direct_tcpip(host.to_string(), port as u32, "127.0.0.1", 0)
            .await
            .map_err(|error| AppError::with_detail(ErrorCode::ForwardFailed, error.to_string()))
    }

    pub(crate) async fn tcpip_forward(&self, host: &str, port: u16) -> Result<u32, AppError> {
        self.inner
            .tcpip_forward(host.to_string(), port as u32)
            .await
            .map_err(|error| AppError::with_detail(ErrorCode::ForwardFailed, error.to_string()))
    }

    pub(crate) async fn cancel_tcpip_forward(&self, host: &str, port: u16) -> Result<(), AppError> {
        self.unregister_remote_target(port);
        self.inner
            .cancel_tcpip_forward(host.to_string(), port as u32)
            .await
            .map_err(|error| AppError::with_detail(ErrorCode::ForwardFailed, error.to_string()))
    }

    pub(crate) fn register_remote_target(&self, port: u16, host: String, target_port: u16) {
        if let Ok(mut map) = self.remote_forwards.lock() {
            map.insert(port, (host, target_port));
        }
    }

    pub(crate) fn unregister_remote_target(&self, port: u16) {
        if let Ok(mut map) = self.remote_forwards.lock() {
            map.remove(&port);
        }
        self.forward_tasks.abort_port(port);
    }

    /// 执行命令并返回标准输出，忽略退出码与错误输出。
    pub async fn exec_command(&self, command: &str) -> Result<String, AppError> {
        Ok(self.exec_command_capture(command).await?.stdout)
    }

    /// 执行命令并完整捕获标准输出、错误输出与远端退出码。
    pub async fn exec_command_capture(&self, command: &str) -> Result<CommandOutput, AppError> {
        match tokio::time::timeout(EXEC_COMMAND_TIMEOUT, self.exec_command_collect(command)).await {
            Ok(result) => result,
            Err(_) => Err(AppError::with_detail(
                ErrorCode::Timeout,
                format!(
                    "远端命令执行超时，已等待 {} 秒，命令: {}",
                    EXEC_COMMAND_TIMEOUT.as_secs(),
                    command_excerpt(command)
                ),
            )),
        }
    }

    async fn exec_command_collect(&self, command: &str) -> Result<CommandOutput, AppError> {
        let mut channel = self.open_session().await?;
        channel
            .exec(true, command)
            .await
            .map_err(|error| AppError::with_detail(ErrorCode::ChannelFailed, error.to_string()))?;

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut exit_status = None;
        while let Some(msg) = channel.wait().await {
            match msg {
                ChannelMsg::Data { data } => {
                    if exec_output_exceeds(&stdout, &stderr, data.len()) {
                        let _ = channel.close().await;
                        return Err(exec_output_limit_error(command));
                    }
                    stdout.extend_from_slice(&data);
                }
                ChannelMsg::ExtendedData { data, ext: 1 } => {
                    if exec_output_exceeds(&stdout, &stderr, data.len()) {
                        let _ = channel.close().await;
                        return Err(exec_output_limit_error(command));
                    }
                    stderr.extend_from_slice(&data);
                }
                ChannelMsg::ExitStatus {
                    exit_status: status,
                } => exit_status = Some(status),
                ChannelMsg::Eof | ChannelMsg::Close => break,
                _ => {}
            }
        }
        Ok(CommandOutput {
            exit_status,
            stdout: String::from_utf8_lossy(&stdout).to_string(),
            stderr: String::from_utf8_lossy(&stderr).to_string(),
        })
    }

    pub async fn disconnect(&self, description: &str) {
        self.forward_tasks.abort_all();
        let _ = self
            .inner
            .disconnect(Disconnect::ByApplication, description, "en")
            .await;
        for handle in self.jump_handles.iter().rev() {
            let _ = handle
                .disconnect(Disconnect::ByApplication, description, "en")
                .await;
        }
    }
}

pub(crate) fn exec_output_exceeds(stdout: &[u8], stderr: &[u8], incoming: usize) -> bool {
    stdout
        .len()
        .saturating_add(stderr.len())
        .saturating_add(incoming)
        > MAX_EXEC_OUTPUT_BYTES
}

pub(crate) fn exec_output_limit_error(command: &str) -> AppError {
    AppError::with_detail(
        ErrorCode::Protocol,
        format!(
            "远端命令输出超过 {} 字节上限，已中止读取，命令: {}",
            MAX_EXEC_OUTPUT_BYTES,
            command_excerpt(command)
        ),
    )
}

pub(crate) fn command_excerpt(command: &str) -> String {
    if command.len() <= COMMAND_EXCERPT_BYTES {
        return command.to_string();
    }
    let mut end = COMMAND_EXCERPT_BYTES;
    while !command.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &command[..end])
}

pub struct SshSession {
    pub io: TerminalIo,
    pub join: JoinHandle<()>,
    pub remote: SshRemote,
    /// 整条 SSH 连接的传输层累计流量，供状态栏采样。
    pub traffic: Arc<TrafficCounter>,
}

pub(crate) async fn run_terminal_actor(
    remote: SshRemote,
    mut channel: Channel<Msg>,
    input_rx: flume::Receiver<Vec<u8>>,
    mut control_rx: tokio::sync::mpsc::Receiver<TerminalControl>,
    output_tx: flume::Sender<Vec<u8>>,
) {
    enum Activity {
        Channel(Option<ChannelMsg>),
        Input(Result<Vec<u8>, flume::RecvError>),
        Control(Option<TerminalControl>),
    }

    loop {
        let activity = tokio::select! {
            message = channel.wait() => Activity::Channel(message),
            input = input_rx.recv_async() => Activity::Input(input),
            control = control_rx.recv() => Activity::Control(control),
        };

        match activity {
            Activity::Channel(Some(ChannelMsg::Data { data })) => {
                if output_tx.send_async(data.to_vec()).await.is_err() {
                    break;
                }
            }
            Activity::Channel(Some(ChannelMsg::ExtendedData { data, .. })) => {
                if output_tx.send_async(data.to_vec()).await.is_err() {
                    break;
                }
            }
            Activity::Channel(Some(ChannelMsg::Eof))
            | Activity::Channel(Some(ChannelMsg::Close)) => {
                break;
            }
            Activity::Channel(Some(ChannelMsg::ExitStatus { .. }))
            | Activity::Channel(Some(ChannelMsg::ExitSignal { .. }))
            | Activity::Channel(None) => break,
            Activity::Channel(Some(_)) => {}
            Activity::Input(Ok(bytes)) => {
                if channel.data_bytes(bytes).await.is_err() {
                    break;
                }
            }
            Activity::Input(Err(_)) => break,
            Activity::Control(Some(TerminalControl::Resize(size))) => {
                if let Err(error) = channel
                    .window_change(size.cols as u32, size.rows as u32, 0, 0)
                    .await
                {
                    log::warn!(
                        "同步远端 PTY 窗口尺寸失败，远端 shell 将以旧尺寸排版，画面可能持续错位，列数: {}，行数: {}，原因: {error}",
                        size.cols,
                        size.rows
                    );
                }
            }
            Activity::Control(Some(TerminalControl::Close)) => {
                let _ = channel.eof().await;
                let _ = channel.close().await;
                break;
            }
            Activity::Control(None) => break,
        }
    }

    remote.disconnect("terminal closed").await;
}
