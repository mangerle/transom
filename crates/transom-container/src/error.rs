//! 容器接入层的错误类型与到 [`AppError`] 的映射。
//!
//! 设计原理（Why）：
//! - 底层错误必须保留端点、操作与状态码等现场信息，才能区分「服务没启动」
//!   与「权限不足」这两条完全不同的恢复路径；
//! - 跨 crate 边界时收敛为 `AppError` 的稳定错误码，界面不解析本枚举；
//! - 本层只构造错误，不记录日志：错误由控制器在最终消费点统一记录，
//!   避免同一个失败在调用链里被打印多次。

use transom_core::{AppError, ErrorCode};

/// 容器接入层错误。
#[derive(Debug, thiserror::Error)]
pub enum ContainerError {
    /// 无法建立到引擎端点的连接。
    #[error("连接容器引擎失败，端点: {endpoint}，原因: {source}")]
    Connect {
        endpoint: String,
        #[source]
        source: std::io::Error,
    },
    /// HTTP 交互本身失败（握手、读写、Upgrade）。
    #[error("容器引擎 HTTP 交互失败，操作: {operation}，原因: {message}")]
    Http { operation: String, message: String },
    /// 引擎返回非成功状态码。
    #[error("容器引擎返回错误，操作: {operation}，状态码: {status}，响应: {body}")]
    Api {
        operation: String,
        status: u16,
        body: String,
    },
    /// 目标容器、镜像或文件不存在。
    #[error("容器引擎中找不到目标: {target}")]
    NotFound { target: String },
    /// 当前后端或引擎版本不支持该操作。
    #[error("容器引擎不支持该操作: {operation}")]
    Unsupported { operation: String },
    /// 容器内路径不合法（非绝对路径、含 `..` 或控制字符）。
    #[error("容器路径非法: {path}，原因: {reason}")]
    InvalidPath { path: String, reason: String },
    /// 容器文件读写失败。
    #[error("容器文件操作失败，路径: {path}，原因: {message}")]
    File { path: String, message: String },
    /// 本机没有发现可用引擎。
    #[error("本机未发现可用的容器引擎（Docker/Podman）：{detail}")]
    NoEngine { detail: String },
    /// 请求被用户取消。
    #[error("容器操作已取消")]
    Cancelled,
    /// 响应解析失败。
    #[error("解析容器引擎响应失败，操作: {operation}，原因: {message}")]
    Decode { operation: String, message: String },
    /// 本地子进程（CLI 兜底）失败。
    #[error("容器 CLI 执行失败，命令: {command}，原因: {message}")]
    Cli { command: String, message: String },
}

impl ContainerError {
    /// 映射到跨层稳定的错误码。
    pub fn to_app_error(&self) -> AppError {
        let code = match self {
            Self::Connect { .. } | Self::NoEngine { .. } | Self::Cli { .. } => {
                ErrorCode::ContainerUnavailable
            }
            Self::NotFound { .. } => ErrorCode::ContainerNotFound,
            Self::Api { status, .. } if *status == 404 => ErrorCode::ContainerNotFound,
            Self::Api { status, .. } if *status == 409 || *status == 304 => {
                ErrorCode::ContainerConflict
            }
            Self::Api { .. } | Self::Http { .. } | Self::Decode { .. } => ErrorCode::ContainerApi,
            Self::Unsupported { .. } | Self::InvalidPath { .. } => ErrorCode::Configuration,
            Self::File { .. } => ErrorCode::Io,
            Self::Cancelled => ErrorCode::Cancelled,
        };
        AppError::with_detail(code, self.to_string())
    }
}

impl From<ContainerError> for AppError {
    fn from(value: ContainerError) -> Self {
        value.to_app_error()
    }
}

#[cfg(test)]
mod tests {
    use super::ContainerError;
    use transom_core::ErrorCode;

    #[test]
    fn not_found_status_maps_to_not_found_code() {
        let error = ContainerError::Api {
            operation: "inspect".into(),
            status: 404,
            body: "no such container".into(),
        };
        assert_eq!(error.to_app_error().code, ErrorCode::ContainerNotFound);
    }

    #[test]
    fn conflict_status_maps_to_conflict_code() {
        let error = ContainerError::Api {
            operation: "remove".into(),
            status: 409,
            body: "container is running".into(),
        };
        assert_eq!(error.to_app_error().code, ErrorCode::ContainerConflict);
    }

    #[test]
    fn invalid_path_is_configuration_error() {
        let error = ContainerError::InvalidPath {
            path: "../etc".into(),
            reason: "必须是绝对路径".into(),
        };
        assert_eq!(error.to_app_error().code, ErrorCode::Configuration);
    }
}
