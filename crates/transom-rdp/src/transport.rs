//! RDP 网络传输层拨号与超时保护。
//!
//! 设计原理（Why）：RDP 连接建立阶段最容易受到远端网络防火墙、路由黑洞或
//! 套接字挂起的影响。所有建立连接操作必须显式包裹 `tokio::time::timeout`
//! 并监听会话取消信号，杜绝因对端静默丢包引发协程永久挂死。

use crate::error::RdpError;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::sync::watch;

/// 网络连接超时限制（秒）。
const DEFAULT_CONNECT_TIMEOUT_SECS: u64 = 15;

/// 建立到指定目标主机的异步 TCP 流（支持超时与停机取消）。
///
/// # Errors
/// - 拨号失败返回 [`RdpError::Network`]；
/// - 超过超时限制返回 [`RdpError::Timeout`]；
/// - 收到上层取消信号返回 [`RdpError::ChannelClosed`]。
pub async fn connect_stream(
    host: &str,
    port: u16,
    shutdown_rx: &mut watch::Receiver<bool>,
) -> Result<TcpStream, RdpError> {
    if *shutdown_rx.borrow() {
        return Err(RdpError::ChannelClosed {
            channel: "RDP 会话停机信号",
        });
    }

    let target = format!("{host}:{port}");
    let timeout_duration = Duration::from_secs(DEFAULT_CONNECT_TIMEOUT_SECS);

    let connect_fut = tokio::time::timeout(timeout_duration, TcpStream::connect(&target));
    tokio::select! {
        _ = shutdown_rx.changed() => {
            Err(RdpError::ChannelClosed {
                channel: "RDP 会话停机信号",
            })
        }
        result = connect_fut => {
            match result {
                Ok(Ok(stream)) => {
                    // 设置 TCP_NODELAY 降低桌面操作往返时延
                    if let Err(e) = stream.set_nodelay(true) {
                        log::debug!("设置 RDP TCP_NODELAY 失败，忽略继续: {e}");
                    }
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
    }
}
