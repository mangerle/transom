//! SSH2 客户端核心封装：多跳跳板机隧道、多策略认证与会话通道生命周期。

mod auth;
mod handler;
mod jump;
mod session;

#[cfg(test)]
mod tests;

use crate::auth::AuthPrompter;
use crate::bridge::{ChannelReader, ChannelWriter};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use transom_core::{
    AppError, ConnectionSecrets, ErrorCode, KnownHostsStore, OutboundProxy, SshConnection, SshHop,
    TerminalControlHandle, TerminalIo, TerminalSize,
};

pub use session::{CommandOutput, SshRemote, SshSession};

/// SSH 建连参数。
///
/// 参数超过五个后收敛为结构体：跳板链、凭据、提示器与 known_hosts 都是
/// "本次连接一次性"的输入，平铺会让调用点难以核对顺序与遗漏。
pub struct SshConnectParams<'a> {
    pub connection: &'a SshConnection,
    /// 已解析的跳板链，顺序为最外层到最内层；空表示直连。
    pub jumps: &'a [SshHop],
    pub size: TerminalSize,
    pub credentials: ConnectionSecrets,
    pub prompter: Arc<dyn AuthPrompter>,
    pub known_hosts: Arc<dyn KnownHostsStore>,
    /// 会话实际生效的出站代理（已按"会话优先于全局"解析）。
    pub proxy: OutboundProxy,
    /// 是否允许 SHA1（`ssh-rsa`）做用户公钥认证：false 时遇到只支持
    /// SHA1 的服务器直接拒绝；true 时按兼容模式继续并告警留痕。
    pub allow_sha1_publickey_auth: bool,
}

pub async fn connect_ssh(params: SshConnectParams<'_>) -> Result<SshSession, AppError> {
    let SshConnectParams {
        connection,
        jumps,
        size,
        credentials,
        prompter,
        known_hosts,
        proxy,
        allow_sha1_publickey_auth,
    } = params;
    if connection.host.trim().is_empty() {
        return Err(AppError::with_detail(
            ErrorCode::Configuration,
            "主机地址为空",
        ));
    }
    if connection.username.trim().is_empty() {
        return Err(AppError::with_detail(
            ErrorCode::Configuration,
            "用户名为空",
        ));
    }

    let remote_forwards = Arc::new(Mutex::new(HashMap::new()));
    let (session, traffic, jump_handles) = jump::establish_chain(jump::ChainParams {
        target: connection,
        jumps,
        credentials,
        prompter: prompter.clone(),
        known_hosts,
        remote_forwards: remote_forwards.clone(),
        proxy,
        allow_sha1_publickey_auth,
    })
    .await?;

    let channel = session
        .channel_open_session()
        .await
        .map_err(|error| AppError::with_detail(ErrorCode::ChannelFailed, error.to_string()))?;

    channel
        .request_pty(
            false,
            "xterm-256color",
            size.cols as u32,
            size.rows as u32,
            0,
            0,
            &[],
        )
        .await
        .map_err(|error| AppError::with_detail(ErrorCode::ChannelFailed, error.to_string()))?;
    let _ = channel.set_env(false, "COLORTERM", "truecolor").await;
    channel
        .request_shell(false)
        .await
        .map_err(|error| AppError::with_detail(ErrorCode::ChannelFailed, error.to_string()))?;

    let (input_tx, input_rx) = flume::bounded::<Vec<u8>>(64);
    let (output_tx, output_rx) = flume::bounded::<Vec<u8>>(128);
    let (control, control_rx) = TerminalControlHandle::channel(16);
    let remote = SshRemote::new(Arc::new(session), remote_forwards, jump_handles);

    let join = tokio::spawn(session::run_terminal_actor(
        remote.clone(),
        channel,
        input_rx,
        control_rx,
        output_tx,
    ));

    let io = TerminalIo::new(
        Box::new(ChannelReader::new(output_rx)),
        Box::new(ChannelWriter::new(input_tx)),
        control,
    );

    Ok(SshSession {
        io,
        join,
        remote,
        traffic,
    })
}
