//! 本地存储领域错误：区分损坏、版本过高、权限、原子替换与序列化失败。
//!
//! 设计原理（Why）：
//! - 存储故障的恢复路径差异很大：损坏文件需要隔离，版本过高必须升级程序，
//!   权限问题需要修正目录，原子替换失败则要保留旧文件并提示重试；
//! - 领域枚举保留路径、版本与操作现场，再转换为稳定的事件错误 DTO；
//! - 底层 `serde_json`、平台文件替换和系统 IO 错误只作为原因文本或 `source`
//!   保存在本层，不通过 `#[from]` 暴露给上层。

use std::io;
use transom_core::{AppError, ErrorCode, ErrorContext};

/// 本地持久化操作的强类型失败。
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    /// 文件内容无法按当前格式解析。
    #[error("存储文件内容损坏，路径: {path}，原因: {reason}")]
    Corrupt { path: String, reason: String },
    /// 文件格式版本高于当前程序支持范围。
    #[error("存储文件版本过高，路径: {path}，文件版本: {found}，支持上限: {supported}")]
    SchemaTooNew {
        path: String,
        found: u64,
        supported: u64,
    },
    /// 文件系统拒绝访问。
    #[error("存储文件权限不足，操作: {operation}，路径: {path}，原因: {source}")]
    Permission {
        operation: String,
        path: String,
        #[source]
        source: io::Error,
    },
    /// 临时文件替换正式文件失败。
    #[error("原子替换存储文件失败，路径: {path}，原因: {source}")]
    AtomicReplace {
        path: String,
        #[source]
        source: io::Error,
    },
    /// 序列化待保存内容失败。
    #[error("序列化存储内容失败，路径: {path}，原因: {reason}")]
    Serialization { path: String, reason: String },
    /// 路径没有可用父目录或不符合存储约束。
    #[error("存储路径非法，路径: {path}，原因: {reason}")]
    InvalidPath { path: String, reason: String },
    /// 读取文件失败。
    #[error("读取存储文件失败，路径: {path}，原因: {source}")]
    Read {
        path: String,
        #[source]
        source: io::Error,
    },
    /// 写入、同步或创建目录失败。
    #[error("写入存储文件失败，操作: {operation}，路径: {path}，原因: {source}")]
    Write {
        operation: String,
        path: String,
        #[source]
        source: io::Error,
    },
}

impl StorageError {
    fn code(&self) -> ErrorCode {
        match self {
            Self::Corrupt { .. } => ErrorCode::StorageCorrupt,
            Self::SchemaTooNew { .. } => ErrorCode::StorageSchemaTooNew,
            Self::Serialization { .. } | Self::InvalidPath { .. } => ErrorCode::Configuration,
            Self::Permission { .. }
            | Self::AtomicReplace { .. }
            | Self::Read { .. }
            | Self::Write { .. } => ErrorCode::Io,
        }
    }

    fn context(&self) -> ErrorContext {
        let (operation, path, reason) = match self {
            Self::Corrupt { path, reason } => {
                ("解析存储文件".to_string(), path.clone(), reason.clone())
            }
            Self::SchemaTooNew {
                path,
                found,
                supported,
            } => (
                "检查存储版本".to_string(),
                path.clone(),
                format!("文件版本 {found} 高于支持上限 {supported}"),
            ),
            Self::Permission {
                operation,
                path,
                source,
            } => (operation.clone(), path.clone(), source.to_string()),
            Self::AtomicReplace { path, source } => (
                "原子替换存储文件".to_string(),
                path.clone(),
                source.to_string(),
            ),
            Self::Serialization { path, reason } => {
                ("序列化存储内容".to_string(), path.clone(), reason.clone())
            }
            Self::InvalidPath { path, reason } => {
                ("校验存储路径".to_string(), path.clone(), reason.clone())
            }
            Self::Read { path, source } => {
                ("读取存储文件".to_string(), path.clone(), source.to_string())
            }
            Self::Write {
                operation,
                path,
                source,
            } => (operation.clone(), path.clone(), source.to_string()),
        };
        ErrorContext::Storage {
            operation,
            path,
            reason,
        }
    }
}

impl From<StorageError> for AppError {
    fn from(error: StorageError) -> Self {
        let code = error.code();
        let context = error.context();
        AppError::with_context(code, error.to_string(), context)
    }
}

#[cfg(test)]
mod tests {
    use super::StorageError;
    use transom_core::{AppError, ErrorCode, ErrorContext};

    /// 损坏文件必须保留独立错误码，调用方可触发隔离而不是静默空启动。
    #[test]
    fn corrupt_file_maps_to_storage_corrupt() {
        let error = StorageError::Corrupt {
            path: "/tmp/settings.json".to_string(),
            reason: "缺少右花括号".to_string(),
        };
        let app: AppError = error.into();
        assert_eq!(app.code, ErrorCode::StorageCorrupt);
        assert!(matches!(
            app.context(),
            Some(ErrorContext::Storage { path, .. }) if path == "/tmp/settings.json"
        ));
    }

    /// 版本过高必须与损坏分开，避免旧程序覆盖新版本数据。
    #[test]
    fn newer_schema_maps_to_schema_too_new() {
        let error = StorageError::SchemaTooNew {
            path: "/tmp/sessions.json".to_string(),
            found: 9,
            supported: 3,
        };
        assert_eq!(AppError::from(error).code, ErrorCode::StorageSchemaTooNew);
    }
}
