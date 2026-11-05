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
    /// 容器或镜像名称不符合引擎命名规则。
    #[error("名称非法: {name}，原因: {reason}")]
    InvalidName { name: String, reason: String },
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
            Self::Unsupported { .. } | Self::InvalidPath { .. } | Self::InvalidName { .. } => {
                ErrorCode::Configuration
            }
            Self::File { .. } => ErrorCode::Io,
            Self::Cancelled => ErrorCode::Cancelled,
        };
        AppError::with_detail(code, self.user_friendly_detail())
    }

    /// 面向用户的友好错误详情，去除裸 JSON、内部状态码等非必要技术细节。
    pub fn user_friendly_detail(&self) -> String {
        match self {
            Self::Api {
                operation,
                status,
                body,
            } => humanize_api_error(operation, *status, body),
            _ => self.to_string(),
        }
    }
}

/// 解析并提炼容器引擎 API 错误为直观易懂的中文诊断说明。
fn humanize_api_error(operation: &str, status: u16, body: &str) -> String {
    let raw_msg = if let Ok(v) = serde_json::from_str::<serde_json::Value>(body) {
        v.get("message")
            .and_then(|m| m.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| body.trim().to_string())
    } else {
        body.trim().to_string()
    };

    // 常见容器冲突与错误场景模式匹配
    if raw_msg.contains("image is being used by running container") {
        if let Some(pos) = raw_msg.find("running container ") {
            let rest = &raw_msg[pos + "running container ".len()..];
            let id = rest
                .split_whitespace()
                .next()
                .unwrap_or(rest)
                .trim_matches('"');
            let short = if id.len() > 12 { &id[..12] } else { id };
            return format!("镜像正被运行中的容器 ({short}) 使用，无法删除");
        }
        return "镜像正被运行中的容器使用，无法删除".to_string();
    }
    if raw_msg.contains("image is being used by stopped container") {
        if let Some(pos) = raw_msg.find("stopped container ") {
            let rest = &raw_msg[pos + "stopped container ".len()..];
            let id = rest
                .split_whitespace()
                .next()
                .unwrap_or(rest)
                .trim_matches('"');
            let short = if id.len() > 12 { &id[..12] } else { id };
            return format!("镜像正被已停止的容器 ({short}) 使用，请先删除容器或强制删除");
        }
        return "镜像正被已停止的容器使用，请先删除容器或强制删除".to_string();
    }
    if raw_msg.contains("cannot remove a running container")
        || raw_msg.contains("Stop the container before attempting removal")
    {
        return "容器正在运行中，无法直接删除，请先停止容器或强制删除".to_string();
    }
    if raw_msg.contains("is already in use") {
        return "该资源已被其它容器或服务使用，无法删除".to_string();
    }
    if raw_msg.contains("port is already allocated") {
        return "指定的端口已被占用，请修改端口映射配置".to_string();
    }
    if raw_msg.contains("has active endpoints") {
        return "该网络下仍有活跃容器连接，无法删除".to_string();
    }
    if raw_msg.contains("No such container") {
        return "目标容器不存在或已被删除".to_string();
    }
    if raw_msg.contains("No such image") {
        return "目标镜像不存在或已被删除".to_string();
    }
    if raw_msg.contains("No such volume") {
        return "目标数据卷不存在或已被删除".to_string();
    }
    if raw_msg.contains("No such network") {
        return "目标网络不存在或已被删除".to_string();
    }

    if !raw_msg.is_empty() {
        if status == 409 {
            return format!("操作冲突：{raw_msg}");
        }
        return raw_msg;
    }

    format!("容器引擎返回错误（操作: {operation}，状态码: {status}）")
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
    fn invalid_name_is_configuration_error() {
        let error = ContainerError::InvalidName {
            name: "_bad name".into(),
            reason: "必须以字母或数字开头".into(),
        };
        assert_eq!(error.to_app_error().code, ErrorCode::Configuration);
    }

    #[test]
    fn invalid_path_is_configuration_error() {
        let error = ContainerError::InvalidPath {
            path: "../etc".into(),
            reason: "必须是绝对路径".into(),
        };
        assert_eq!(error.to_app_error().code, ErrorCode::Configuration);
    }

    #[test]
    fn humanizes_docker_image_conflict_message() {
        let raw_json = r#"{"message":"conflict: unable to delete d9e853e87e55 (cannot be forced) - image is being used by running container ab0dd73ed23d"}"#;
        let error = ContainerError::Api {
            operation: "删除镜像".into(),
            status: 409,
            body: raw_json.into(),
        };
        let app_error = error.to_app_error();
        assert_eq!(app_error.code, ErrorCode::ContainerConflict);
        let detail = app_error.detail.unwrap_or_default();
        assert!(detail.contains("镜像正被运行中的容器"));
        assert!(detail.contains("ab0dd73ed23d"));
    }
}
