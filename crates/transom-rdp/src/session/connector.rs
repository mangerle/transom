//! RDP 连接序列驱动：拨号、TLS 升级与 NLA / CredSSP 认证。
//!
//! 设计原理（Why）：将握手连接序列与主数据传输循环严格解耦。连接阶段各步骤
//! （X.224、MCS、CredSSP、Capabilities Exchange）依次流转，若任意步骤失败，
//! 立即将底层错误转换为具体的 `RdpError`，避免向外泄漏第三方内部易变类型。

use crate::error::RdpError;
use crate::settings::SessionConfig;
use crate::transport::connect_stream;
use ironrdp_connector::{ClientConnector, ConnectionResult, ServerName};
use ironrdp_tokio::reqwest::ReqwestNetworkClient;
use ironrdp_tokio::{TokioFramed, connect_begin, connect_finalize, mark_as_upgraded};
use tokio::io::{AsyncRead, AsyncWrite};

/// 统一的流对象擦除抽象。
pub type ErasedStream = Box<dyn AsyncReadWrite + Unpin + Send + Sync>;

/// 读写异步特型组合。
pub trait AsyncReadWrite: AsyncRead + AsyncWrite {}
impl<T: AsyncRead + AsyncWrite> AsyncReadWrite for T {}

/// 已建立并就绪的 RDP 会话握手上下文。
pub struct EstablishedConnection {
    pub framed: TokioFramed<ErasedStream>,
    pub result: ConnectionResult,
}

/// 执行完整的 RDP 连接与认证序列。
pub async fn establish_connection(
    config: &SessionConfig,
) -> Result<EstablishedConnection, RdpError> {
    // 1. 建立基础 TCP 传输流
    let stream = connect_stream(&config.host, config.port).await?;
    let local_addr = stream.local_addr().map_err(|e| RdpError::Network {
        address: config.host.clone(),
        source: e,
    })?;

    let mut framed: TokioFramed<ErasedStream> = TokioFramed::new(Box::new(stream));
    let rdp_config = config.to_ironrdp_config();
    let mut connector = ClientConnector::new(rdp_config, local_addr);

    // 2. 执行 connect_begin 握手
    let should_upgrade = connect_begin(&mut framed, &mut connector)
        .await
        .map_err(|e| RdpError::Protocol {
            stage: "connect_begin 握手初始化",
            detail: e.to_string(),
        })?;

    // 3. 执行 TLS 安全升级与公钥提取
    let (initial_stream, leftover) = framed.into_inner();
    let (tls_stream, certificate) = ironrdp_tls::upgrade(initial_stream, &config.host)
        .await
        .map_err(|e| RdpError::Tls {
            host: config.host.clone(),
            message: e.to_string(),
        })?;

    let server_public_key = ironrdp_tls::extract_tls_server_public_key(&certificate)
        .ok_or_else(|| RdpError::Tls {
            host: config.host.clone(),
            message: "未能从服务器 TLS 证书提取公钥".to_string(),
        })?
        .to_owned();

    // 4. 执行 connect_finalize 驱动 CredSSP / NLA 认证
    let upgraded = mark_as_upgraded(should_upgrade, &mut connector);
    let erased: ErasedStream = Box::new(tls_stream);
    let mut framed = TokioFramed::new_with_leftover(erased, leftover);

    let outcome = connect_finalize(
        upgraded,
        connector,
        &mut framed,
        &mut ReqwestNetworkClient::new(),
        ServerName::new(&config.host),
        server_public_key,
        None,
    )
    .await
    .map_err(|e| RdpError::Authentication {
        username: config.username.clone(),
        message: e.to_string(),
    })?;

    let result = outcome.into_connected().map_err(|e| RdpError::Protocol {
        stage: "connect_finalize 会话就绪确认",
        detail: e.to_string(),
    })?;

    Ok(EstablishedConnection { framed, result })
}
