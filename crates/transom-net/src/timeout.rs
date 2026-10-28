//! 外部等待的统一超时口径。
//!
//! 设计原理（Why）：控制平面运行在固定两个工作线程的 tokio 运行时上，任何
//! 一次无上限的远端等待都会永久占住一个 worker，进而拖住全部连接与停机
//! 流程。ssh2 握手之后的通道打开、PTY/Shell 请求、SFTP 子系统初始化与
//! 转发登记同样是外部等待，必须与拨号阶段一样有明确上限。
//!
//! 实现上只统一"等待多久、超时怎么报"，不改变各调用点原有的错误码归类：
//! 底层失败沿用调用方传入的错误码，真正超时统一归为 [`ErrorCode::Timeout`]，
//! 便于界面区分"对端拒绝"与"对端无响应"。
//!
//! 代价与局限：超时按操作类型固定分档，慢链路（高延迟跳板、拥塞网络）
//! 可能需要更长；当前不提供用户级配置，后续如有诉求再按连接选项扩展。

use std::fmt::Display;
use std::future::Future;
use std::time::Duration;
use transom_core::{AppError, ErrorCode};

/// 打开 SSH 通道的上限：通道打开包含一次完整的往返确认。
pub(crate) const CHANNEL_OPEN_TIMEOUT: Duration = Duration::from_secs(20);
/// PTY、Shell 等通道请求的上限：通常比通道打开快。
pub(crate) const CHANNEL_REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
/// 转发登记/注销等轻量请求的上限。
pub(crate) const FORWARD_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// 主动断开连接的上限：停机路径不能被失联的对端拖住。
pub(crate) const DISCONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// SFTP 子系统初始化（协议版本交换与主目录查询）的上限。
pub(crate) const SFTP_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(20);
/// SOCKS5 本机协商（问候、请求与目标通道建立）的上限。
///
/// 只约束握手阶段；CONNECT/UDP 中继是长连接，不能设置总时限。
pub(crate) const SOCKS5_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);

/// 给单个外部等待套上超时，并统一中文错误描述。
///
/// `code` 只用于底层失败；超时一律返回 [`ErrorCode::Timeout`]，让界面能把
/// "对端无响应"与"对端明确拒绝"分成两条恢复路径。
pub(crate) async fn with_timeout<T, E, F>(
    duration: Duration,
    code: ErrorCode,
    action: &str,
    future: F,
) -> Result<T, AppError>
where
    E: Display,
    F: Future<Output = Result<T, E>>,
{
    match tokio::time::timeout(duration, future).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(AppError::with_detail(
            code,
            format!("{action}失败，原因: {error}"),
        )),
        Err(_) => Err(AppError::with_detail(
            ErrorCode::Timeout,
            format!("{action}超时，等待上限: {} 秒", duration.as_secs()),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::with_timeout;
    use std::io;
    use std::time::Duration;
    use transom_core::ErrorCode;

    /// 超时必须归入 Timeout，且带上操作名与等待上限，便于界面提示。
    #[tokio::test]
    async fn timeout_maps_to_timeout_code_with_action() {
        let error = with_timeout(
            Duration::from_millis(5),
            ErrorCode::ChannelFailed,
            "打开 SSH 会话通道",
            async {
                tokio::time::sleep(Duration::from_millis(50)).await;
                Ok::<(), io::Error>(())
            },
        )
        .await
        .expect_err("休眠长于超时上限，必须返回超时错误");
        assert_eq!(error.code, ErrorCode::Timeout);
        let text = error.to_string();
        assert!(text.contains("打开 SSH 会话通道"));
        assert!(text.contains("等待上限"));
    }

    /// 底层失败保留调用方错误码，不被超时逻辑吞掉。
    #[tokio::test]
    async fn underlying_error_keeps_caller_code() {
        let error = with_timeout(
            Duration::from_secs(1),
            ErrorCode::ForwardFailed,
            "登记远端转发",
            async { Err::<(), _>(io::Error::other("被拒绝")) },
        )
        .await
        .expect_err("底层失败必须向上返回");
        assert_eq!(error.code, ErrorCode::ForwardFailed);
        assert!(error.to_string().contains("被拒绝"));
    }
}
