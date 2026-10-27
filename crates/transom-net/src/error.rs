//! 网络层细粒度错误：连接、握手、主机密钥、认证与通道域的强类型失败。
//!
//! 设计原理（Why）：
//! - 实现初衷：全工程共用的 `AppError { code, detail }` 中 detail 是裸字符串，
//!   调用方无法按失败类别模式匹配；连接链路（拨号/握手/认证/主机密钥）是
//!   排查断连问题的主要入口，率先收敛为 thiserror 枚举；
//! - 核心优势：变体携带动态现场（主机、端口、原因），Display 全中文自带
//!   上下文；到 `AppError` 的错误码映射集中在一处，不再散落各调用点；
//! - 代价与局限：SFTP 与转发子域仍使用 `AppError`（其 detail 已中文化、
//!   错误码已细分），后续按需迁移；russh 等第三方错误只作为 `source`
//!   存储、不做 `#[from]`，避免外部依赖变动破坏本层契约。

use transom_core::{AppError, ErrorCode};

/// 连接链路的细粒度错误；经 `From` 转换为全工程 `AppError` 后跨层传递。
#[derive(Debug, thiserror::Error)]
pub enum NetError {
    /// TCP 拨号失败：携带目标主机与端口。
    #[error("连接远端节点失败，主机: {host}，端口: {port}，原因: {source}")]
    Connect {
        host: String,
        port: u16,
        #[source]
        source: std::io::Error,
    },
    /// SSH 握手、版本协商或会话建立失败（含连接超时）。
    #[error("SSH 会话建立失败，主机: {host}，端口: {port}，原因: {reason}")]
    Handshake {
        host: String,
        port: u16,
        reason: String,
    },
    /// 主机密钥校验未通过（用户拒绝或与已记录指纹不一致）。
    #[error("主机密钥校验未通过，主机: {host}，端口: {port}，原因: {reason}")]
    HostKey {
        host: String,
        port: u16,
        reason: String,
    },
    /// 认证失败：reason 描述具体环节（密码、私钥、agent、交互式）。
    #[error("SSH 认证失败: {reason}")]
    Auth { reason: String },
    /// 出站代理握手失败：携带代理地址与目标，reason 为协议层原因（不含口令）。
    #[error("经出站代理建立隧道失败，代理: {proxy}，目标: {target}，原因: {reason}")]
    Proxy {
        proxy: String,
        target: String,
        reason: String,
    },
    /// 通道打开或子系统请求失败。
    #[error("打开 SSH 通道失败: {reason}")]
    Channel { reason: String },
    /// 用户取消（关闭弹窗、凭据输入为空等）。
    #[error("操作已取消: {reason}")]
    Cancelled { reason: String },
    /// 主机拨测超时：端口在给定时间内没有完成三次握手。
    #[error("探测远端端口超时，主机: {host}，端口: {port}，等待: {timeout_ms} 毫秒")]
    ProbeTimeout {
        host: String,
        port: u16,
        timeout_ms: u64,
    },
    /// Wake-on-LAN 魔法包发送失败（MAC 非法、无广播路由或套接字不可用）。
    #[error("发送 Wake-on-LAN 魔法包失败，网卡: {mac}，原因: {reason}")]
    Wake { mac: String, reason: String },
}

impl NetError {
    /// 到全工程错误码的映射；与 UI 层本地化文案键一一对应。
    fn code(&self) -> ErrorCode {
        match self {
            Self::Connect { .. } | Self::Handshake { .. } | Self::Proxy { .. } => {
                ErrorCode::ConnectionFailed
            }
            Self::HostKey { .. } => ErrorCode::HostKeyMismatch,
            Self::Auth { .. } => ErrorCode::AuthFailed,
            Self::Channel { .. } => ErrorCode::ChannelFailed,
            Self::Cancelled { .. } => ErrorCode::Cancelled,
            Self::ProbeTimeout { .. } => ErrorCode::Timeout,
            // 唤醒失败是"本机没能把包发出去"，归到 IO 而不是连接失败：
            // 目标机器状态未知，不能被误读成对端拒绝。
            Self::Wake { .. } => ErrorCode::Io,
        }
    }
}

impl From<NetError> for AppError {
    fn from(error: NetError) -> Self {
        AppError::with_detail(error.code(), error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::NetError;

    /// 错误码映射与中文现场：跨层传递后仍可按 code 归类、文本可读。
    #[test]
    fn net_error_maps_to_app_error_codes() {
        let error = NetError::Connect {
            host: "web-01".to_string(),
            port: 22,
            source: std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "refused"),
        };
        let app_error = transom_core::AppError::from(error);
        assert_eq!(app_error.code, transom_core::ErrorCode::ConnectionFailed);
        assert!(app_error.to_string().contains("web-01"));

        let error = NetError::Auth {
            reason: "服务器拒绝了全部认证方式".to_string(),
        };
        assert_eq!(
            transom_core::AppError::from(error).code,
            transom_core::ErrorCode::AuthFailed
        );

        // 拨测超时归 Timeout，与"连不上"区分开，界面可单独提示改长超时。
        let error = NetError::ProbeTimeout {
            host: "10.0.0.8".to_string(),
            port: 22,
            timeout_ms: 2000,
        };
        assert_eq!(
            transom_core::AppError::from(error).code,
            transom_core::ErrorCode::Timeout
        );

        // 唤醒失败是本地发送问题，不能报成对端连接失败。
        let error = NetError::Wake {
            mac: "aa:bb:cc:dd:ee:ff".to_string(),
            reason: "network is unreachable".to_string(),
        };
        let app_error = transom_core::AppError::from(error);
        assert_eq!(app_error.code, transom_core::ErrorCode::Io);
        assert!(app_error.to_string().contains("aa:bb:cc:dd:ee:ff"));
    }
}
