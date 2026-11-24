//! RDP 连接序列驱动：拨号、TLS 升级与 NLA / CredSSP 认证。
//!
//! 设计原理（Why）：将握手连接序列与主数据传输循环严格解耦。连接阶段各步骤
//! （X.224、MCS、CredSSP、Capabilities Exchange）依次流转。对每个网络与握手
//! 步骤施加显式超时保护，防止远端黑洞导致协程永久泄漏；按需挂载 RDPSND 等静态通道；
//! 若任意步骤失败，立即将底层错误转换为具体的 `RdpError`。

use crate::audio::TransomAudioBackend;
use crate::clipboard::{CliprdrSyncContext, TransomCliprdrBackend};
use crate::error::RdpError;
use crate::session::channel::RdpOutput;
use crate::settings::SessionConfig;
use crate::transport::connect_stream;
use ironrdp_connector::{ClientConnector, ConnectionResult, ServerName};
use ironrdp_tokio::reqwest::ReqwestNetworkClient;
use ironrdp_tokio::{TokioFramed, connect_begin, connect_finalize, mark_as_upgraded};
use parking_lot::Mutex;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc;

/// 单步握手与安全协商超时限制（秒）。
const DEFAULT_HANDSHAKE_TIMEOUT_SECS: u64 = 15;

/// 统一的流对象擦除抽象。
pub type ErasedStream = Box<dyn AsyncReadWrite + Unpin + Send + Sync>;

/// 读写异步特型组合。
pub trait AsyncReadWrite: AsyncRead + AsyncWrite {}
impl<T: AsyncRead + AsyncWrite> AsyncReadWrite for T {}

/// 已建立并就绪的 RDP 会话握手上下文。
pub struct EstablishedConnection {
    pub framed: TokioFramed<ErasedStream>,
    pub result: ConnectionResult,
    pub audio_muted: Arc<AtomicBool>,
    pub cliprdr_context: Arc<Mutex<CliprdrSyncContext>>,
}

/// 执行完整的 RDP 连接与认证序列。
pub async fn establish_connection(
    config: &SessionConfig,
    output_tx: &mpsc::Sender<RdpOutput>,
) -> Result<EstablishedConnection, RdpError> {
    // 1. 建立基础 TCP 传输流
    let stream = connect_stream(&config.host, config.port).await?;
    let local_addr = stream.local_addr().map_err(|e| RdpError::Network {
        address: config.host.clone(),
        source: e,
    })?;

    let audio_muted = Arc::new(AtomicBool::new(!config.audio));
    let cliprdr_context = Arc::new(Mutex::new(CliprdrSyncContext::default()));

    let mut framed: TokioFramed<ErasedStream> = TokioFramed::new(Box::new(stream));
    let mut connector = build_client_connector(
        config,
        local_addr,
        audio_muted.clone(),
        cliprdr_context.clone(),
        output_tx.clone(),
    );

    // 2. 执行 connect_begin 握手
    let should_upgrade = tokio::time::timeout(
        Duration::from_secs(DEFAULT_HANDSHAKE_TIMEOUT_SECS),
        connect_begin(&mut framed, &mut connector),
    )
    .await
    .map_err(|_| RdpError::Timeout {
        stage: "connect_begin 握手初始化",
        timeout_secs: DEFAULT_HANDSHAKE_TIMEOUT_SECS,
    })?
    .map_err(|e| RdpError::Protocol {
        stage: "connect_begin 握手初始化",
        detail: e.to_string(),
    })?;

    // 3. 执行 TLS 安全升级与公钥提取
    let (initial_stream, leftover) = framed.into_inner();
    let (tls_stream, server_public_key) = perform_tls_upgrade(initial_stream, &config.host).await?;

    // 4. 执行 connect_finalize 驱动 CredSSP / NLA 认证
    let upgraded = mark_as_upgraded(should_upgrade, &mut connector);
    let erased: ErasedStream = Box::new(tls_stream);
    let mut framed = TokioFramed::new_with_leftover(erased, leftover);
    let mut network_client = ReqwestNetworkClient::new();

    let outcome = tokio::time::timeout(
        Duration::from_secs(DEFAULT_HANDSHAKE_TIMEOUT_SECS),
        connect_finalize(
            upgraded,
            connector,
            &mut framed,
            &mut network_client,
            ServerName::new(&config.host),
            server_public_key,
            None,
        ),
    )
    .await
    .map_err(|_| RdpError::Timeout {
        stage: "connect_finalize 身份认证与通道确认",
        timeout_secs: DEFAULT_HANDSHAKE_TIMEOUT_SECS,
    })?
    .map_err(|e| RdpError::Authentication {
        username: config.username.clone(),
        message: e.to_string(),
    })?;

    let result = outcome.into_connected().map_err(|e| RdpError::Protocol {
        stage: "connect_finalize 会话就绪确认",
        detail: e.to_string(),
    })?;

    Ok(EstablishedConnection {
        framed,
        result,
        audio_muted,
        cliprdr_context,
    })
}

/// 构建包含能力标志与静态通道的客户端连接器。
fn build_client_connector(
    config: &SessionConfig,
    local_addr: SocketAddr,
    audio_muted: Arc<AtomicBool>,
    cliprdr_context: Arc<Mutex<CliprdrSyncContext>>,
    output_tx: mpsc::Sender<RdpOutput>,
) -> ClientConnector {
    let rdp_config = config.to_ironrdp_config();
    let mut connector = ClientConnector::new(rdp_config, local_addr);

    // 若启用了音频重定向，挂载 RDPSND 静态通道
    if config.audio {
        let backend = TransomAudioBackend::new(audio_muted);
        connector =
            connector.with_static_channel(ironrdp_rdpsnd::client::Rdpsnd::new(Box::new(backend)));
    }

    // 若启用了剪贴板重定向，挂载 CLIPRDR 静态通道
    if config.clipboard {
        let backend = TransomCliprdrBackend::new(cliprdr_context, output_tx);
        connector = connector.with_static_channel(ironrdp_cliprdr::Cliprdr::new(Box::new(backend)));
    }

    // 挂载 DRDYNVC 动态虚拟通道，并注册 Display Control 动态通道（打通远端动态重设分辨率协议闭环）
    let drdynvc = ironrdp_dvc::DrdynvcClient::new().with_dynamic_channel(
        ironrdp_displaycontrol::client::DisplayControlClient::new(|_| Ok(Vec::new())),
    );
    connector = connector.with_static_channel(drdynvc);

    connector
}

/// 执行 TLS 通道升级并提取服务器公钥。
async fn perform_tls_upgrade<S>(
    stream: S,
    host: &str,
) -> Result<(ironrdp_tls::TlsStream<S>, Vec<u8>), RdpError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
{
    let (tls_stream, certificate) = tokio::time::timeout(
        Duration::from_secs(DEFAULT_HANDSHAKE_TIMEOUT_SECS),
        ironrdp_tls::upgrade(stream, host),
    )
    .await
    .map_err(|_| RdpError::Timeout {
        stage: "TLS 安全握手升级",
        timeout_secs: DEFAULT_HANDSHAKE_TIMEOUT_SECS,
    })?
    .map_err(|e| RdpError::Tls {
        host: host.to_string(),
        message: e.to_string(),
    })?;

    let public_key = ironrdp_tls::extract_tls_server_public_key(&certificate)
        .ok_or_else(|| RdpError::Tls {
            host: host.to_string(),
            message: "未能从服务器 TLS 证书提取公钥".to_string(),
        })?
        .to_owned();

    Ok((tls_stream, public_key))
}
