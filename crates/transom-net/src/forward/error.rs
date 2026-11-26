//! 端口转发领域错误：区分绑定失败、目标不可达、超时与 SOCKS 协议错误。
//!
//! 设计原理（Why）：
//! - 转发失败常由不同层产生，但恢复动作并不相同：本地端口占用要换端口，
//!   目标不可达要看网络/服务，SOCKS 协商失败要检查客户端协议；
//! - 领域枚举把操作名、绑定地址、目标和超时保存为动态现场，再转换到事件
//!   边界使用的 `AppError`，UI 不再依赖错误文案判断下一步；
//! - 外部 IO 错误只作为 `source` 保存，不通过 `#[from]` 泄漏为稳定契约。

use std::io;
use transom_core::{AppError, ErrorCode, ErrorContext};

/// 端口转发操作的强类型失败。
#[derive(Debug, thiserror::Error)]
pub enum ForwardError {
    /// 本机监听端口绑定失败。
    #[error("绑定转发端口失败，地址: {bind}，原因: {source}")]
    BindFailed {
        bind: String,
        #[source]
        source: io::Error,
    },
    /// 固定目标地址不可达。
    #[error("转发目标不可达，目标: {target}，原因: {reason}")]
    TargetUnreachable { target: String, reason: String },
    /// 目标地址配置非法。
    #[error("转发目标配置非法，目标: {target}，原因: {reason}")]
    InvalidTarget { target: String, reason: String },
    /// 转发探测或 SOCKS 协商超时。
    #[error("转发操作超时，操作: {operation}，目标: {target}，等待: {timeout_ms} 毫秒")]
    Timeout {
        operation: String,
        target: String,
        timeout_ms: u64,
    },
    /// 用户或上层任务取消转发。
    #[error("转发操作已取消，操作: {operation}，原因: {reason}")]
    Cancelled { operation: String, reason: String },
    /// SOCKS5 或转发协议不符合契约。
    #[error("转发协议错误，操作: {operation}，原因: {reason}")]
    Protocol { operation: String, reason: String },
    /// 当前转发类型不支持该操作。
    #[error("当前转发类型不支持该操作，操作: {operation}，原因: {reason}")]
    Unsupported { operation: String, reason: String },
    /// 转发链路中的本地 IO 失败。
    #[error("转发本地 IO 失败，操作: {operation}，原因: {source}")]
    Io {
        operation: String,
        #[source]
        source: io::Error,
    },
}

impl ForwardError {
    fn code(&self) -> ErrorCode {
        match self {
            Self::BindFailed { .. }
            | Self::TargetUnreachable { .. }
            | Self::InvalidTarget { .. }
            | Self::Cancelled { .. } => match self {
                Self::Cancelled { .. } => ErrorCode::Cancelled,
                _ => ErrorCode::ForwardFailed,
            },
            Self::Timeout { .. } => ErrorCode::Timeout,
            Self::Protocol { .. } => ErrorCode::Protocol,
            Self::Unsupported { .. } => ErrorCode::NotSupported,
            Self::Io { .. } => ErrorCode::Io,
        }
    }

    fn context(&self) -> ErrorContext {
        let (operation, target, reason) = match self {
            Self::BindFailed { bind, source } => {
                ("绑定转发端口".to_string(), bind.clone(), source.to_string())
            }
            Self::TargetUnreachable { target, reason } => {
                ("连接转发目标".to_string(), target.clone(), reason.clone())
            }
            Self::InvalidTarget { target, reason } => {
                ("校验转发目标".to_string(), target.clone(), reason.clone())
            }
            Self::Timeout {
                operation,
                target,
                timeout_ms,
            } => (
                operation.clone(),
                target.clone(),
                format!("等待 {timeout_ms} 毫秒"),
            ),
            Self::Cancelled { operation, reason } => {
                (operation.clone(), String::new(), reason.clone())
            }
            Self::Protocol { operation, reason } => {
                (operation.clone(), String::new(), reason.clone())
            }
            Self::Unsupported { operation, reason } => {
                (operation.clone(), String::new(), reason.clone())
            }
            Self::Io { operation, source } => {
                (operation.clone(), String::new(), source.to_string())
            }
        };
        ErrorContext::Forward {
            operation,
            target,
            reason,
        }
    }
}

impl From<ForwardError> for AppError {
    fn from(error: ForwardError) -> Self {
        let code = error.code();
        let context = error.context();
        AppError::with_context(code, error.to_string(), context)
    }
}

#[cfg(test)]
mod tests {
    use super::ForwardError;
    use transom_core::{AppError, ErrorCode, ErrorContext};

    /// 绑定失败保留地址，并且不会被误报成目标不可达。
    #[test]
    fn bind_failure_keeps_address_context() {
        let error = ForwardError::BindFailed {
            bind: "127.0.0.1:8080".to_string(),
            source: std::io::Error::new(std::io::ErrorKind::AddrInUse, "占用"),
        };
        let app: AppError = error.into();
        assert_eq!(app.code, ErrorCode::ForwardFailed);
        assert!(matches!(
            app.context(),
            Some(ErrorContext::Forward { target, .. }) if target == "127.0.0.1:8080"
        ));
    }

    /// 超时使用独立错误码，调用方可以提示调整等待上限。
    #[test]
    fn timeout_maps_to_timeout_code() {
        let error = ForwardError::Timeout {
            operation: "探测目标".to_string(),
            target: "10.0.0.8:443".to_string(),
            timeout_ms: 3000,
        };
        let app: AppError = error.into();
        assert_eq!(app.code, ErrorCode::Timeout);
        assert!(app.to_string().contains("10.0.0.8:443"));
    }
}
