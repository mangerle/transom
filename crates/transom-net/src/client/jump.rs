//! 多跳 ProxyJump 隧道级联构建与出站代理穿透。

use super::auth::authenticate;
use super::handler::{
    AsyncStream, ClientError, ClientHandler, REMOTE_FORWARD_LIMIT, RemoteForwardTasks,
};
use crate::auth::AuthPrompter;
use crate::bridge::AsyncChannelStream;
use crate::error::NetError;
use crate::traffic::CountingStream;
use russh::client::{self, Handle};
use russh::keys::Algorithm;
use russh::{Disconnect, Preferred};
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio::sync::Semaphore;
use transom_core::{
    AppError, ConnectionSecrets, ErrorCode, KnownHostsStore, OutboundProxy, SessionOptions,
    SshConnection, SshHop, TrafficCounter,
};

/// 建连链路的完整输入：跳板链、目标、凭据与各跳共享的认证设施。
pub(crate) struct ChainParams<'a> {
    pub(crate) target: &'a SshConnection,
    pub(crate) jumps: &'a [SshHop],
    pub(crate) credentials: ConnectionSecrets,
    pub(crate) prompter: Arc<dyn AuthPrompter>,
    pub(crate) known_hosts: Arc<dyn KnownHostsStore>,
    /// 目标会话的远端转发登记表；与 `ClientHandler` 共享同一个 Arc。
    pub(crate) remote_forwards: Arc<Mutex<HashMap<u16, (String, u16)>>>,
    /// 远端转发任务追踪器；在会话断开或取消转发时收割任务。
    pub(crate) forward_tasks: RemoteForwardTasks,
    /// 会话实际生效的出站代理；只作用于最外层物理连接。
    pub(crate) proxy: OutboundProxy,
    /// 是否允许 SHA1（`ssh-rsa`）用户公钥认证；跳板机同样适用。
    pub(crate) allow_sha1_publickey_auth: bool,
}

/// 单跳握手与认证的输入。
pub(crate) struct HandshakeParams<'a> {
    pub(crate) connection: &'a SshConnection,
    pub(crate) credentials: ConnectionSecrets,
    pub(crate) prompter: Arc<dyn AuthPrompter>,
    pub(crate) known_hosts: Arc<dyn KnownHostsStore>,
    pub(crate) config: Arc<client::Config>,
    pub(crate) remote_forwards: Arc<Mutex<HashMap<u16, (String, u16)>>>,
    pub(crate) forward_tasks: RemoteForwardTasks,
    pub(crate) allow_sha1_publickey_auth: bool,
}

/// 逐跳建链过程中的累积状态。
pub(crate) struct ChainBuild<'a> {
    pub(crate) params: ChainParams<'a>,
    pub(crate) config: Arc<client::Config>,
    pub(crate) traffic: Arc<TrafficCounter>,
}

/// 协商偏好：保留 russh 的现代默认值，但不接受 SHA1 的 `ssh-rsa` 主机签名。
pub(crate) fn ssh_preferred() -> Preferred {
    let mut preferred = Preferred::DEFAULT;
    preferred.key = Cow::Owned(
        preferred
            .key
            .iter()
            .filter(|algorithm| !matches!(algorithm, Algorithm::Rsa { hash: None }))
            .cloned()
            .collect(),
    );
    preferred
}

/// 建立"TCP（或代理）-> 各跳板机 SSH -> 目标 SSH"的完整链路。
pub(crate) async fn establish_chain(
    params: ChainParams<'_>,
) -> Result<
    (
        Handle<ClientHandler>,
        Arc<TrafficCounter>,
        Vec<Arc<Handle<ClientHandler>>>,
    ),
    AppError,
> {
    let config = Arc::new(client::Config {
        nodelay: true,
        window_size: 16 * 1024 * 1024,
        keepalive_interval: keepalive_interval(&params.target.options),
        keepalive_max: 3,
        preferred: ssh_preferred(),
        ..Default::default()
    });
    let traffic = Arc::new(TrafficCounter::default());
    let (result, jump_handles) = build_chain(ChainBuild {
        params,
        config,
        traffic: traffic.clone(),
    })
    .await;
    match result {
        Ok(session) => Ok((session, traffic, jump_handles)),
        Err(error) => {
            disconnect_handles(&jump_handles, "跳板链建立失败").await;
            Err(error)
        }
    }
}

async fn build_chain(
    build: ChainBuild<'_>,
) -> (
    Result<Handle<ClientHandler>, AppError>,
    Vec<Arc<Handle<ClientHandler>>>,
) {
    let ChainBuild {
        params:
            ChainParams {
                target,
                jumps,
                credentials,
                prompter,
                known_hosts,
                remote_forwards,
                forward_tasks,
                proxy,
                allow_sha1_publickey_auth,
            },
        config,
        traffic,
    } = build;
    let mut jump_handles = Vec::with_capacity(jumps.len());
    let dial_target = jumps.first().map_or(target, |hop| &hop.connection);
    let outer_timeout = dial_target.options.connect_timeout_secs.max(1);
    let transport = match tokio::time::timeout(
        Duration::from_secs(outer_timeout),
        open_transport(dial_target, &proxy),
    )
    .await
    {
        Ok(Ok(transport)) => transport,
        Ok(Err(error)) => return (Err(error), jump_handles),
        Err(_) => {
            let error = NetError::Connect {
                host: dial_target.host.clone(),
                port: dial_target.port,
                source: std::io::Error::new(std::io::ErrorKind::TimedOut, "连接阶段超时"),
            };
            return (Err(error.into()), jump_handles);
        }
    };
    let mut transport: Box<dyn AsyncStream> = Box::new(transport);

    for (index, hop) in jumps.iter().enumerate() {
        let handle = match establish_session(
            transport,
            HandshakeParams {
                connection: &hop.connection,
                credentials: hop.secrets.clone(),
                prompter: prompter.clone(),
                known_hosts: known_hosts.clone(),
                config: config.clone(),
                remote_forwards: Arc::new(Mutex::new(HashMap::new())),
                forward_tasks: RemoteForwardTasks::default(),
                allow_sha1_publickey_auth,
            },
        )
        .await
        {
            Ok(handle) => handle,
            Err(error) => {
                return (
                    Err(AppError::with_detail(
                        error.code,
                        format!(
                            "经跳板机 {}:{} 建连失败: {}",
                            hop.connection.host,
                            hop.connection.port,
                            error.detail.as_deref().unwrap_or("未知原因")
                        ),
                    )),
                    jump_handles,
                );
            }
        };
        let (next_host, next_port) = jumps
            .get(index + 1)
            .map_or((&target.host, target.port), |next| {
                (&next.connection.host, next.connection.port)
            });
        let channel = handle
            .channel_open_direct_tcpip(next_host.clone(), next_port as u32, "127.0.0.1", 0)
            .await;
        let channel = match channel {
            Ok(channel) => channel,
            Err(error) => {
                jump_handles.push(Arc::new(handle));
                let error = NetError::Channel {
                    reason: format!(
                        "经跳板机 {}:{} 打开到 {next_host}:{next_port} 的直连通道失败: {error}",
                        hop.connection.host, hop.connection.port
                    ),
                };
                return (Err(error.into()), jump_handles);
            }
        };
        log::debug!(
            "跳板机通道已建立，跳板机: {}:{}，下一跳: {next_host}:{next_port}",
            hop.connection.host,
            hop.connection.port
        );
        transport = Box::new(AsyncChannelStream::new(channel));
        jump_handles.push(Arc::new(handle));
    }

    let final_transport = CountingStream::new(transport, traffic);
    let result = establish_session(
        final_transport,
        HandshakeParams {
            connection: target,
            credentials,
            prompter,
            known_hosts,
            config,
            remote_forwards,
            forward_tasks,
            allow_sha1_publickey_auth,
        },
    )
    .await;
    (result, jump_handles)
}

async fn open_transport(
    connection: &SshConnection,
    proxy: &OutboundProxy,
) -> Result<TcpStream, AppError> {
    let socket = crate::proxy::connect_via_proxy(&connection.host, connection.port, proxy).await?;
    if let Err(error) = socket.set_nodelay(true) {
        log::debug!("设置 TCP_NODELAY 失败，将继续使用默认行为，原因: {error}");
    }
    Ok(socket)
}

/// 在已建立的传输上完成 SSH 握手与认证。
///
/// 设计原理（Why）：这里不套总超时。`connect_stream` 内部包含主机密钥确认，
/// `authenticate` 会等待用户输入密码或验证码；把它们计入同一个截止时间会让
/// 正常操作被误判超时。TCP 拨号超时由 [`build_chain`] 覆盖，握手或认证挂起
/// 时用户可以直接取消连接，任务随标签关闭而中止。
async fn establish_session<S>(
    transport: S,
    params: HandshakeParams<'_>,
) -> Result<Handle<ClientHandler>, AppError>
where
    S: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    let HandshakeParams {
        connection,
        credentials,
        prompter,
        known_hosts,
        config,
        remote_forwards,
        forward_tasks,
        allow_sha1_publickey_auth,
    } = params;
    let handler = ClientHandler {
        host: connection.host.clone(),
        port: connection.port,
        known_hosts,
        remote_forwards,
        forward_tasks,
        remote_forward_slots: Arc::new(Semaphore::new(REMOTE_FORWARD_LIMIT)),
        prompter: prompter.clone(),
    };
    let mut session = client::connect_stream(config, transport, handler)
        .await
        .map_err(map_connect_error)?;
    authenticate(
        &mut session,
        connection,
        credentials,
        prompter,
        allow_sha1_publickey_auth,
    )
    .await?;
    Ok(session)
}

pub(crate) async fn disconnect_handles(handles: &[Arc<Handle<ClientHandler>>], reason: &str) {
    for handle in handles.iter().rev() {
        let _ = handle
            .disconnect(Disconnect::ByApplication, reason, "en")
            .await;
    }
}

fn keepalive_interval(options: &SessionOptions) -> Option<Duration> {
    (options.keepalive_secs > 0).then(|| Duration::from_secs(options.keepalive_secs))
}

fn map_connect_error(error: ClientError) -> AppError {
    if matches!(
        error.0.code,
        ErrorCode::HostKeyMismatch | ErrorCode::HostKeyUnknown
    ) {
        error.0
    } else {
        AppError::with_detail(ErrorCode::ConnectionFailed, error.0.to_string())
    }
}
