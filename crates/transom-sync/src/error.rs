//! 云同步领域错误：将配置、认证、冲突、超限与传输故障分开表达。
//!
//! 设计原理（Why）：
//! - 同步冲突必须触发“重新拉取、合并、重试”，而普通网络失败应直接结束；
//!   只用 `CloudSyncFailed` 加字符串会迫使 UI 解析文案；
//! - 枚举保留端点、操作与动态原因，转换到 `AppError` 时仍可序列化为事件 DTO；
//! - 请求库错误不会通过 `#[from]` 暴露，避免第三方升级改变同步模块的公开契约。

use transom_core::{AppError, ErrorCode, ErrorContext};

/// 云同步操作的强类型失败。
#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    /// 远端尚无同步对象。
    #[error("云端还没有同步备份，端点: {endpoint}")]
    Missing { endpoint: String },
    /// 写入因远端版本变化被拒绝。
    #[error("云端同步冲突，端点: {endpoint}，对象: {object}，原因: {reason}")]
    Conflict {
        endpoint: String,
        object: String,
        reason: String,
    },
    /// 认证信息被端点拒绝。
    #[error("云端认证失败，端点: {endpoint}，操作: {operation}，原因: {reason}")]
    Unauthorized {
        endpoint: String,
        operation: String,
        reason: String,
    },
    /// 远端文档超过允许大小。
    #[error("云端文档超过上限，端点: {endpoint}，实际: {actual}，上限: {limit}")]
    TooLarge {
        endpoint: String,
        actual: u64,
        limit: u64,
    },
    /// 同步配置缺失或字段非法。
    #[error("云同步配置无效，字段: {field}，原因: {reason}")]
    InvalidConfiguration { field: String, reason: String },
    /// 网络传输失败。
    #[error("云同步传输失败，端点: {endpoint}，操作: {operation}，原因: {reason}")]
    Transport {
        endpoint: String,
        operation: String,
        reason: String,
    },
    /// 请求等待超时。
    #[error("云同步请求超时，端点: {endpoint}，操作: {operation}，等待: {timeout_ms} 毫秒")]
    Timeout {
        endpoint: String,
        operation: String,
        timeout_ms: u64,
    },
    /// 加解密、序列化或远端响应解析失败。
    #[error("云同步数据处理失败，操作: {operation}，原因: {reason}")]
    Decode { operation: String, reason: String },
    /// 阻塞任务异常终止。
    #[error("云同步后台任务失败，操作: {operation}，原因: {reason}")]
    TaskFailed { operation: String, reason: String },
}

impl SyncError {
    fn code(&self) -> ErrorCode {
        match self {
            Self::Missing { .. } => ErrorCode::CloudSyncMissing,
            Self::Conflict { .. } => ErrorCode::CloudSyncConflict,
            Self::Unauthorized { .. } => ErrorCode::AuthFailed,
            Self::TooLarge { .. } | Self::Transport { .. } | Self::Decode { .. } => {
                ErrorCode::CloudSyncFailed
            }
            Self::Timeout { .. } => ErrorCode::Timeout,
            Self::InvalidConfiguration { .. } => ErrorCode::Configuration,
            Self::TaskFailed { .. } => ErrorCode::CloudSyncFailed,
        }
    }

    fn context(&self) -> ErrorContext {
        let (endpoint, operation, reason) = match self {
            Self::Missing { endpoint } => (
                endpoint.clone(),
                "检查远端文档".to_string(),
                "远端对象不存在".to_string(),
            ),
            Self::Conflict {
                endpoint,
                object,
                reason,
            } => (
                endpoint.clone(),
                format!("写入对象 {object}"),
                reason.clone(),
            ),
            Self::Unauthorized {
                endpoint,
                operation,
                reason,
            } => (endpoint.clone(), operation.clone(), reason.clone()),
            Self::TooLarge {
                endpoint,
                actual,
                limit,
            } => (
                endpoint.clone(),
                "读取远端文档".to_string(),
                format!("实际 {actual}，上限 {limit}"),
            ),
            Self::InvalidConfiguration { field, reason } => {
                (String::new(), format!("校验配置 {field}"), reason.clone())
            }
            Self::Transport {
                endpoint,
                operation,
                reason,
            } => (endpoint.clone(), operation.clone(), reason.clone()),
            Self::Timeout {
                endpoint,
                operation,
                timeout_ms,
            } => (
                endpoint.clone(),
                operation.clone(),
                format!("等待 {timeout_ms} 毫秒"),
            ),
            Self::Decode { operation, reason } => {
                (String::new(), operation.clone(), reason.clone())
            }
            Self::TaskFailed { operation, reason } => {
                (String::new(), operation.clone(), reason.clone())
            }
        };
        ErrorContext::Sync {
            endpoint,
            operation,
            reason,
        }
    }
}

impl From<SyncError> for AppError {
    fn from(error: SyncError) -> Self {
        let code = error.code();
        let context = error.context();
        AppError::with_context(code, error.to_string(), context)
    }
}

#[cfg(test)]
mod tests {
    use super::SyncError;
    use transom_core::{AppError, ErrorCode, ErrorContext};

    /// 冲突必须保留独立错误码，供同步面板进入合并流程。
    #[test]
    fn conflict_maps_to_conflict_code() {
        let error = SyncError::Conflict {
            endpoint: "https://sync.example.com/transom".to_string(),
            object: "transom.sync".to_string(),
            reason: "版本不匹配".to_string(),
        };
        let app: AppError = error.into();
        assert_eq!(app.code, ErrorCode::CloudSyncConflict);
        assert!(matches!(
            app.context(),
            Some(ErrorContext::Sync { endpoint, .. })
                if endpoint == "https://sync.example.com/transom"
        ));
    }
}
