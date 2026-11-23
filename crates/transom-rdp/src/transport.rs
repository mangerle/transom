//! RDP 网络传输层拨号与代理流通道。
//!
//! 设计原理（Why）：RDP 连接建立阶段最容易受到远端网络防火墙、路由黑洞或
//! 套接字挂起的影响。所有建立连接操作必须显式包裹 `tokio::time::timeout`，
//! 杜绝因对端静默丢包引发协程永久挂死。

use crate::error::RdpError;
use std::time::Duration;
use tokio::net::TcpStream;

/// 网络连接超时限制（秒）。
const DEFAULT_CONNECT_TIMEOUT_SECS: u64 = 15;

/// 建立到指定目标主机的异步 TCP 流。
///
/// 外部 I/O 强制设限超时，保证连接阶段具有确定的时延界限。
pub async fn connect_stream(host: &str, port: u16) -> Result<TcpStream, RdpError> {
    let target = format!("{host}:{port}");
    let timeout_duration = Duration::from_secs(DEFAULT_CONNECT_TIMEOUT_SECS);

    let connect_future = TcpStream::connect(&target);
    match tokio::time::timeout(timeout_duration, connect_future).await {
        Ok(Ok(stream)) => {
            // 设置 TCP_NODELAY 降低桌面操作往返时延
            let _ = stream.set_nodelay(true);
            Ok(stream)
        }
        Ok(Err(source)) => Err(RdpError::Network {
            address: target,
            source,
        }),
        Err(_) => Err(RdpError::Timeout {
            stage: "TCP 拨号连接",
            timeout_secs: DEFAULT_CONNECT_TIMEOUT_SECS,
        }),
    }
}
