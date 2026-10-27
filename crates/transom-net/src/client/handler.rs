//! russh 客户端回调事件处理器与底层传输抽象。

use crate::auth::AuthPrompter;
use crate::error::NetError;
use russh::Channel;
use russh::client::{self, Msg};
use russh::keys::{HashAlg, PublicKeyOrCertificate};
use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::task::JoinHandle;
use transom_core::{AppError, ErrorCode, HostKeyInfo, HostKeyStatus, KnownHostsStore};

/// 传输流抽象：直连 TCP、跳板机通道与出站代理收敛到同一类型。
///
/// 为什么需要它：`russh::client::connect_stream` 只要求 `AsyncRead + AsyncWrite +
/// Send + Unpin + 'static`，用 trait object 把"这一跳的字节从哪来"从握手逻辑里
/// 解耦，裸 TCP 与隧道通道可以走同一段握手/认证代码。
pub(crate) trait AsyncStream: AsyncRead + AsyncWrite + Send + Unpin {}
impl<T: AsyncRead + AsyncWrite + Send + Unpin> AsyncStream for T {}

#[derive(Debug)]
pub struct ClientError(pub AppError);

/// 远端转发建立到目标主机的 TCP 连接的最长等待时间。
///
/// 转发目标通常是内网邻近服务，正常连接远快于该上限；取 30 秒是为
/// 跨机房高延迟链路留余量，同时兜住"对端不可达导致任务永久泄漏"。
pub(crate) const FORWARD_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

impl fmt::Display for ClientError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl std::error::Error for ClientError {}

impl From<russh::Error> for ClientError {
    fn from(error: russh::Error) -> Self {
        Self(AppError::with_detail(
            ErrorCode::Protocol,
            error.to_string(),
        ))
    }
}

impl From<AppError> for ClientError {
    fn from(error: AppError) -> Self {
        Self(error)
    }
}

/// 远端端口转发连接的后台任务追踪器。
///
/// 记录处于活动中的端口转发协程 Handle，支持按远端端口定向中止，
/// 以及在会话断开、取消转发或客户端退场时收割在途的转发任务，彻底消除脱缰孤儿协程。
#[derive(Clone, Default)]
pub(crate) struct RemoteForwardTasks {
    tasks: Arc<Mutex<HashMap<u16, Vec<JoinHandle<()>>>>>,
}

impl RemoteForwardTasks {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// 登记指定远端端口新派生的转发任务句柄，并清理该端口已结束的历史任务。
    pub(crate) fn track(&self, port: u16, handle: JoinHandle<()>) {
        if let Ok(mut lock) = self.tasks.lock() {
            let list = lock.entry(port).or_default();
            list.retain(|h| !h.is_finished());
            list.push(handle);
        }
    }

    /// 取消并收割特定远端转发端口的所有活动中任务。
    pub(crate) fn abort_port(&self, port: u16) {
        if let Ok(mut lock) = self.tasks.lock()
            && let Some(handles) = lock.remove(&port)
        {
            for handle in handles {
                handle.abort();
            }
        }
    }

    /// 取消并收割所有远端转发端口的所有活动中任务。
    pub(crate) fn abort_all(&self) {
        if let Ok(mut lock) = self.tasks.lock() {
            for (_, handles) in lock.drain() {
                for handle in handles {
                    handle.abort();
                }
            }
        }
    }

    /// 统计当前在途的活动中任务总数（供测试与诊断）。
    #[cfg(test)]
    pub(crate) fn active_count(&self) -> usize {
        if let Ok(lock) = self.tasks.lock() {
            lock.values()
                .map(|list| list.iter().filter(|h| !h.is_finished()).count())
                .sum()
        } else {
            0
        }
    }
}

pub(crate) struct ClientHandler {
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) known_hosts: Arc<dyn KnownHostsStore>,
    pub(crate) remote_forwards: Arc<Mutex<HashMap<u16, (String, u16)>>>,
    /// 远端转发在途异步任务追踪器，消除脱缰孤儿任务。
    pub(crate) forward_tasks: RemoteForwardTasks,
    /// TOFU 指纹确认走认证提示通道，与凭据索取共用同一条 UI 往返链路。
    pub(crate) prompter: Arc<dyn AuthPrompter>,
}

impl client::Handler for ClientHandler {
    type Error = ClientError;

    async fn check_server_key(
        &mut self,
        server_key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let public_key = server_key.public_key();
        let fingerprint = public_key.fingerprint(HashAlg::Sha256).to_string();
        let key_type = format!("{:?}", public_key.algorithm());
        let info = HostKeyInfo::new(
            key_type.clone(),
            fingerprint.clone(),
            public_key.to_openssh().ok(),
        );

        match self.known_hosts.check(&self.host, self.port, &info)? {
            HostKeyStatus::Match => Ok(true),
            HostKeyStatus::Unknown => {
                // TOFU：首次连接必须由用户确认指纹后才记录信任，杜绝首连
                // 被 MITM 无感通过；无界面提示器按自身策略自动放行或失败。
                let accepted = self
                    .prompter
                    .confirm_host_key(&self.host, self.port, &key_type, &fingerprint)
                    .await
                    .map_err(ClientError)?;
                if !accepted {
                    return Err(ClientError(AppError::with_detail(
                        ErrorCode::HostKeyUnknown,
                        format!(
                            "用户拒绝了 {}:{} 的主机密钥指纹 {fingerprint}",
                            self.host, self.port
                        ),
                    )));
                }
                self.known_hosts.record(&self.host, self.port, &info)?;
                log::info!(
                    "用户确认后记录新主机密钥，主机: {}，端口: {}，指纹: {fingerprint}",
                    self.host,
                    self.port
                );
                Ok(true)
            }
            HostKeyStatus::Mismatch {
                previous_fingerprint,
            } => Err(ClientError(
                NetError::HostKey {
                    host: self.host.clone(),
                    port: self.port,
                    reason: format!(
                        "主机密钥与已记录指纹不一致，原指纹: {previous_fingerprint}，当前指纹: {fingerprint}"
                    ),
                }
                .into(),
            )),
        }
    }

    async fn server_channel_open_forwarded_tcpip(
        &mut self,
        channel: Channel<Msg>,
        _connected_address: &str,
        connected_port: u32,
        _originator_address: &str,
        _originator_port: u32,
        reply: client::ChannelOpenHandle,
        _session: &mut client::Session,
    ) -> Result<(), Self::Error> {
        let listen_port = connected_port as u16;
        let target = self
            .remote_forwards
            .lock()
            .ok()
            .and_then(|map| map.get(&listen_port).cloned());
        reply.accept().await;
        let Some((host, port)) = target else {
            return Ok(());
        };
        let mut stream = channel.into_stream();
        let forward_task = tokio::spawn(async move {
            // 转发目标是任意外部地址：对端不可达或防火墙静默丢包时 connect 会
            // 无限挂起，任务与通道随之泄漏。只给连接阶段设限，建立后的双向
            // 转发不设限——长空闲本身就是隧道的正常形态。
            let connect = TcpStream::connect((host.as_str(), port));
            match tokio::time::timeout(FORWARD_CONNECT_TIMEOUT, connect).await {
                Ok(Ok(mut tcp)) => {
                    log::debug!(
                        "远端转发通道已连接到本地目标，远端监听端口: {listen_port}，本地目标: {host}:{port}"
                    );
                    let _ = tokio::io::copy_bidirectional(&mut tcp, &mut stream).await;
                }
                Ok(Err(error)) => {
                    log::warn!(
                        "远端转发连接本地目标失败，远端监听端口: {listen_port}，本地目标: {host}:{port}，原因: {error}"
                    );
                    let _ = stream.shutdown().await;
                }
                Err(_) => {
                    log::warn!(
                        "远端转发连接本地目标超时，远端监听端口: {listen_port}，本地目标: {host}:{port}"
                    );
                    let _ = stream.shutdown().await;
                }
            }
        });
        self.forward_tasks.track(listen_port, forward_task);
        Ok(())
    }
}
