//! SFTP 领域错误：把协议状态、路径冲突、体积上限与本地 IO 分开表达。
//!
//! 设计原理（Why）：
//! - `AppError` 的 `detail` 字符串适合跨事件展示，却不适合调用方决定恢复动作；
//!   权限拒绝、路径不存在、版本冲突与超限需要不同的 UI 分支；
//! - 变体保留操作名、路径和超时等现场，转换到 `AppError` 时同时写入结构化上下文；
//! - 第三方 SFTP 错误只在构造时读取，不通过 `#[from]` 泄漏到领域契约，避免依赖升级
//!   直接改变调用方需要匹配的类型。

use std::io;
use transom_core::{AppError, ErrorCode, ErrorContext};

/// SFTP 操作的强类型失败。
#[derive(Debug, thiserror::Error)]
pub enum SftpError {
    /// 远端路径不存在。
    #[error("SFTP 路径不存在，操作: {operation}，路径: {path}")]
    NotFound { operation: String, path: String },
    /// 远端拒绝授权。
    #[error("SFTP 权限不足，操作: {operation}，路径: {path}")]
    PermissionDenied { operation: String, path: String },
    /// 目标状态与本次操作冲突。
    #[error("SFTP 状态冲突，操作: {operation}，路径: {path}，原因: {reason}")]
    Conflict {
        operation: String,
        path: String,
        reason: String,
    },
    /// 文件或预览超过调用方上限。
    #[error("SFTP 文件超过上限，操作: {operation}，路径: {path}，实际: {actual}，上限: {limit}")]
    TooLarge {
        operation: String,
        path: String,
        actual: u64,
        limit: u64,
    },
    /// 本地或远端路径不满足安全约束。
    #[error("SFTP 路径非法，操作: {operation}，路径: {path}，原因: {reason}")]
    InvalidPath {
        operation: String,
        path: String,
        reason: String,
    },
    /// 传输链路或协议客户端报告通用失败。
    #[error("SFTP 传输失败，操作: {operation}，路径: {path}，原因: {reason}")]
    Transport {
        operation: String,
        path: String,
        reason: String,
    },
    /// SFTP 请求等待超时。
    #[error("SFTP 操作超时，操作: {operation}，路径: {path}，等待: {timeout_ms} 毫秒")]
    Timeout {
        operation: String,
        path: String,
        timeout_ms: u64,
    },
    /// 服务端返回不符合协议契约的数据。
    #[error("SFTP 协议错误，操作: {operation}，路径: {path}，原因: {reason}")]
    Protocol {
        operation: String,
        path: String,
        reason: String,
    },
    /// 本地文件系统操作失败。
    #[error("SFTP 本地文件操作失败，操作: {operation}，路径: {path}，原因: {source}")]
    LocalIo {
        operation: String,
        path: String,
        #[source]
        source: io::Error,
    },
}

impl SftpError {
    /// 把 russh-sftp 的错误按协议状态码收敛为领域分类。
    pub(crate) fn from_protocol(
        error: russh_sftp::client::error::Error,
        operation: impl Into<String>,
        path: impl Into<String>,
    ) -> Self {
        let operation = operation.into();
        let path = path.into();
        let reason = error.to_string();
        match &error {
            russh_sftp::client::error::Error::Status(status) => match status.status_code {
                russh_sftp::protocol::StatusCode::NoSuchFile => Self::NotFound { operation, path },
                russh_sftp::protocol::StatusCode::PermissionDenied => {
                    Self::PermissionDenied { operation, path }
                }
                russh_sftp::protocol::StatusCode::Failure => Self::Conflict {
                    operation,
                    path,
                    reason: status.error_message.clone(),
                },
                russh_sftp::protocol::StatusCode::BadMessage => Self::Protocol {
                    operation,
                    path,
                    reason,
                },
                _ => Self::Transport {
                    operation,
                    path,
                    reason,
                },
            },
            russh_sftp::client::error::Error::IO(_) => Self::Transport {
                operation,
                path,
                reason,
            },
            russh_sftp::client::error::Error::Timeout => Self::Timeout {
                operation,
                path,
                timeout_ms: 0,
            },
            russh_sftp::client::error::Error::Limited(_) => Self::TooLarge {
                operation,
                path,
                actual: 0,
                limit: 0,
            },
            russh_sftp::client::error::Error::UnexpectedPacket
            | russh_sftp::client::error::Error::UnexpectedBehavior(_) => Self::Protocol {
                operation,
                path,
                reason,
            },
        }
    }

    /// 把本地 IO 错误包装成带操作与路径的领域错误。
    pub(crate) fn local_io(
        operation: impl Into<String>,
        path: impl Into<String>,
        source: io::Error,
    ) -> Self {
        Self::LocalIo {
            operation: operation.into(),
            path: path.into(),
            source,
        }
    }

    fn code(&self) -> ErrorCode {
        match self {
            Self::NotFound { .. } => ErrorCode::SftpNotFound,
            Self::PermissionDenied { .. } => ErrorCode::PermissionDenied,
            Self::Conflict { .. } => ErrorCode::SftpConflict,
            Self::TooLarge { .. } => ErrorCode::SftpTooLarge,
            Self::InvalidPath { .. } => ErrorCode::Configuration,
            Self::Transport { .. } => ErrorCode::SftpFailed,
            Self::Timeout { .. } => ErrorCode::Timeout,
            Self::Protocol { .. } => ErrorCode::Protocol,
            Self::LocalIo { .. } => ErrorCode::Io,
        }
    }

    fn context(&self) -> ErrorContext {
        let (operation, path, reason) = match self {
            Self::NotFound { operation, path } => (
                operation.clone(),
                path.clone(),
                "远端路径不存在".to_string(),
            ),
            Self::PermissionDenied { operation, path } => {
                (operation.clone(), path.clone(), "远端拒绝授权".to_string())
            }
            Self::Conflict {
                operation,
                path,
                reason,
            } => (operation.clone(), path.clone(), reason.clone()),
            Self::TooLarge {
                operation,
                path,
                actual,
                limit,
            } => (
                operation.clone(),
                path.clone(),
                format!("实际 {actual}，上限 {limit}"),
            ),
            Self::InvalidPath {
                operation,
                path,
                reason,
            } => (operation.clone(), path.clone(), reason.clone()),
            Self::Transport {
                operation,
                path,
                reason,
            } => (operation.clone(), path.clone(), reason.clone()),
            Self::Timeout {
                operation,
                path,
                timeout_ms,
            } => (
                operation.clone(),
                path.clone(),
                format!("等待 {timeout_ms} 毫秒"),
            ),
            Self::Protocol {
                operation,
                path,
                reason,
            } => (operation.clone(), path.clone(), reason.clone()),
            Self::LocalIo {
                operation,
                path,
                source,
            } => (operation.clone(), path.clone(), source.to_string()),
        };
        ErrorContext::Sftp {
            operation,
            path,
            reason,
        }
    }
}

impl From<SftpError> for AppError {
    fn from(error: SftpError) -> Self {
        let code = error.code();
        let context = error.context();
        AppError::with_context(code, error.to_string(), context)
    }
}

#[cfg(test)]
mod tests {
    use super::SftpError;
    use transom_core::{AppError, ErrorCode, ErrorContext};

    /// 协议状态必须映射为可恢复的错误码，而不是统一成字符串。
    #[test]
    fn protocol_status_maps_to_domain_code() {
        let status = russh_sftp::protocol::Status {
            id: 1,
            status_code: russh_sftp::protocol::StatusCode::NoSuchFile,
            error_message: "missing".to_string(),
            language_tag: "en".to_string(),
        };
        let error = SftpError::from_protocol(
            russh_sftp::client::error::Error::Status(status),
            "读取文件",
            "/srv/missing.txt",
        );
        let app: AppError = error.into();
        assert_eq!(app.code, ErrorCode::SftpNotFound);
        assert!(matches!(
            app.context(),
            Some(ErrorContext::Sftp { operation, path, .. })
                if operation == "读取文件" && path == "/srv/missing.txt"
        ));
    }

    /// 超限错误携带实际值与上限，调用方可据此提示具体阻断原因。
    #[test]
    fn too_large_keeps_size_context() {
        let error = SftpError::TooLarge {
            operation: "预览".to_string(),
            path: "/srv/large.log".to_string(),
            actual: 2048,
            limit: 1024,
        };
        let app: AppError = error.into();
        assert_eq!(app.code, ErrorCode::SftpTooLarge);
        assert!(app.to_string().contains("2048"));
        assert!(app.to_string().contains("1024"));
    }
}
