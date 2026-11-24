//! RDP 连接序列驱动：拨号、安全层升级与 NLA / CredSSP 认证。
//!
//! 设计原理（Why）：将握手连接序列与主数据传输循环严格解耦。连接阶段各步骤
//! （X.224、MCS、TLS/CredSSP、Capabilities Exchange）依次流转，并统一施加
//! 超时保护与取消停机监听，防止远端黑洞导致协程泄漏或关闭标签时无法及时退出；
//! 按需挂载 RDPSND、CLIPRDR 与 DisplayControl 通道。

use crate::audio::TransomAudioBackend;
use crate::clipboard::TransomCliprdrBackend;
use crate::error::RdpError;
use crate::session::channel::RdpOutput;
use crate::settings::{SessionConfig, TlsCertVerifier, TlsCertificateInfo};
use crate::transport::connect_stream;
use ironrdp_connector::{ClientConnector, ClientConnectorState, ConnectionResult, ServerName};
use ironrdp_tokio::reqwest::ReqwestNetworkClient;
use ironrdp_tokio::{TokioFramed, connect_begin, connect_finalize, mark_as_upgraded};
use sha2::{Digest, Sha256};
use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{mpsc, watch};

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
}

/// 执行完整的 RDP 连接与认证序列。
///
/// # Errors
/// 拨号失败、握手超时、TLS 升级异常、NLA 认证失败或收到取消停机信号时返回对应 [`RdpError`]。
pub async fn establish_connection(
    config: &SessionConfig,
    events_tx: &mpsc::Sender<RdpOutput>,
    shutdown_rx: &mut watch::Receiver<bool>,
) -> Result<EstablishedConnection, RdpError> {
    let stream = connect_stream(&config.host, config.port, shutdown_rx).await?;
    let local_addr = stream.local_addr().map_err(|e| RdpError::Network {
        address: config.host.clone(),
        source: e,
    })?;

    let audio_muted = Arc::new(AtomicBool::new(!config.audio));
    let mut framed: TokioFramed<ErasedStream> = TokioFramed::new(Box::new(stream));
    let mut connector =
        build_client_connector(config, local_addr, audio_muted.clone(), events_tx.clone());

    let should_upgrade = run_stage(
        "connect_begin 握手初始化",
        shutdown_rx,
        connect_begin(&mut framed, &mut connector),
    )
    .await?
    .map_err(|e| RdpError::Protocol {
        stage: "connect_begin 握手初始化",
        detail: e.to_string(),
    })?;

    let (mut framed, server_public_key) = upgrade_transport_if_needed(
        framed,
        &connector,
        &config.host,
        config.cert_verifier.as_ref(),
        shutdown_rx,
    )
    .await?;
    let result = finalize_authentication(
        should_upgrade,
        connector,
        &mut framed,
        config,
        server_public_key,
        shutdown_rx,
    )
    .await?;

    Ok(EstablishedConnection {
        framed,
        result,
        audio_muted,
    })
}

/// 按协商结果决定是否执行 TLS 升级（标准 RDP 安全模式跳过 TLS）。
async fn upgrade_transport_if_needed(
    framed: TokioFramed<ErasedStream>,
    connector: &ClientConnector,
    host: &str,
    cert_verifier: Option<&TlsCertVerifier>,
    shutdown_rx: &mut watch::Receiver<bool>,
) -> Result<(TokioFramed<ErasedStream>, Vec<u8>), RdpError> {
    let needs_tls = matches!(
        connector.state,
        ClientConnectorState::EnhancedSecurityUpgrade { selected_protocol }
            if !selected_protocol.is_empty()
    );
    if !needs_tls {
        return Ok((framed, Vec::new()));
    }

    let (initial_stream, leftover) = framed.into_inner();
    let (tls_stream, public_key) =
        perform_tls_upgrade(initial_stream, host, cert_verifier, shutdown_rx).await?;
    let erased: ErasedStream = Box::new(tls_stream);
    Ok((TokioFramed::new_with_leftover(erased, leftover), public_key))
}

/// 驱动 CredSSP / NLA 身份认证与通道最终确认。
async fn finalize_authentication(
    should_upgrade: ironrdp_tokio::ShouldUpgrade,
    mut connector: ClientConnector,
    framed: &mut TokioFramed<ErasedStream>,
    config: &SessionConfig,
    server_public_key: Vec<u8>,
    shutdown_rx: &mut watch::Receiver<bool>,
) -> Result<ConnectionResult, RdpError> {
    let upgraded = mark_as_upgraded(should_upgrade, &mut connector);
    let mut network_client = ReqwestNetworkClient::new();

    let outcome = run_stage(
        "connect_finalize 身份认证与通道确认",
        shutdown_rx,
        connect_finalize(
            upgraded,
            connector,
            framed,
            &mut network_client,
            ServerName::new(&config.host),
            server_public_key,
            None,
        ),
    )
    .await?
    .map_err(|e| RdpError::Authentication {
        username: config.username.clone(),
        message: e.to_string(),
    })?;

    outcome.into_connected().map_err(|e| RdpError::Protocol {
        stage: "connect_finalize 会话就绪确认",
        detail: e.to_string(),
    })
}

/// 为异步握手步骤统一施加超时限制与取消信号监听。
async fn run_stage<T, F>(
    stage: &'static str,
    shutdown_rx: &mut watch::Receiver<bool>,
    fut: F,
) -> Result<T, RdpError>
where
    F: Future<Output = T>,
{
    if *shutdown_rx.borrow() {
        return Err(RdpError::ChannelClosed {
            channel: "RDP 会话停机信号",
        });
    }

    let timed = tokio::time::timeout(Duration::from_secs(DEFAULT_HANDSHAKE_TIMEOUT_SECS), fut);
    tokio::select! {
        _ = shutdown_rx.changed() => Err(RdpError::ChannelClosed {
            channel: "RDP 会话停机信号",
        }),
        res = timed => res.map_err(|_| RdpError::Timeout {
            stage,
            timeout_secs: DEFAULT_HANDSHAKE_TIMEOUT_SECS,
        }),
    }
}

/// 构建包含能力标志与按需挂载通道的客户端连接器。
fn build_client_connector(
    config: &SessionConfig,
    local_addr: SocketAddr,
    audio_muted: Arc<AtomicBool>,
    events_tx: mpsc::Sender<RdpOutput>,
) -> ClientConnector {
    let rdp_config = config.to_ironrdp_config();
    let mut connector = ClientConnector::new(rdp_config, local_addr);

    if config.audio {
        let backend = TransomAudioBackend::new(audio_muted);
        connector =
            connector.with_static_channel(ironrdp_rdpsnd::client::Rdpsnd::new(Box::new(backend)));
    }

    if config.clipboard {
        let backend = TransomCliprdrBackend::new(events_tx);
        connector = connector.with_static_channel(ironrdp_cliprdr::Cliprdr::new(Box::new(backend)));
    }

    if config.dynamic_resolution {
        let drdynvc = ironrdp_dvc::DrdynvcClient::new().with_dynamic_channel(
            ironrdp_displaycontrol::client::DisplayControlClient::new(|_| Ok(Vec::new())),
        );
        connector = connector.with_static_channel(drdynvc);
    }

    connector
}

static INIT_CRYPTO_PROVIDER: std::sync::Once = std::sync::Once::new();

/// 确保进程级 rustls CryptoProvider 已安装（工作区同时存在 aws-lc-rs 与 ring 时必须显式指定）。
fn ensure_crypto_provider() {
    INIT_CRYPTO_PROVIDER.call_once(|| {
        if rustls::crypto::CryptoProvider::get_default().is_none() {
            let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        }
    });
}

/// 执行 TLS 通道升级、提取服务器公钥并校验 SHA-256 指纹。
async fn perform_tls_upgrade<S>(
    stream: S,
    host: &str,
    cert_verifier: Option<&TlsCertVerifier>,
    shutdown_rx: &mut watch::Receiver<bool>,
) -> Result<(ironrdp_tls::TlsStream<S>, Vec<u8>), RdpError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
{
    ensure_crypto_provider();
    let (tls_stream, certificate) = run_stage(
        "TLS 安全握手升级",
        shutdown_rx,
        ironrdp_tls::upgrade(stream, host),
    )
    .await?
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

    let fingerprint = format_sha256_fingerprint(&public_key);
    log::info!("RDP TLS 安全通道升级完成，主机: {host}，公钥 SHA-256: {fingerprint}");

    if let Some(verifier) = cert_verifier {
        let info = TlsCertificateInfo {
            host: host.to_string(),
            sha256_fingerprint: fingerprint,
        };
        if let Err(reason) = verifier(&info) {
            return Err(RdpError::Tls {
                host: host.to_string(),
                message: format!("TLS 证书校验未通过: {reason}"),
            });
        }
    }

    Ok((tls_stream, public_key))
}

fn format_sha256_fingerprint(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 3);
    for (idx, b) in digest.iter().enumerate() {
        if idx > 0 {
            out.push(':');
        }
        let _ = write!(&mut out, "{b:02X}");
    }
    out
}
