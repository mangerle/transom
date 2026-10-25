use serde::{Deserialize, Serialize};
use std::fmt;

/// Stable error categories. The UI translates these; `detail` is diagnostic only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ErrorCode {
    Configuration,
    ConnectionFailed,
    Dns,
    Timeout,
    AuthFailed,
    KeyboardInteractiveFailed,
    HostKeyUnknown,
    HostKeyMismatch,
    ChannelFailed,
    SftpFailed,
    /// 编辑回传前发现远端文件已被修改（版本指纹不一致），拒绝覆盖。
    SftpTextConflict,
    ForwardFailed,
    Io,
    SecretStoreUnavailable,
    /// 备份文件是加密信封，但调用方按明文文件处理。
    BackupEncrypted,
    /// 备份文件不是加密信封，但调用方提供了解密口令。
    BackupNotEncrypted,
    Cancelled,
    Protocol,
    /// 云端同步失败：端点不可达、认证被拒、远端拒绝写入或内容超限。
    CloudSyncFailed,
    /// 云端还没有任何备份：首次拉取属预期状态，界面按提示而非故障处理。
    CloudSyncMissing,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppError {
    pub code: ErrorCode,
    pub detail: Option<String>,
}

impl AppError {
    pub fn new(code: ErrorCode) -> Self {
        Self { code, detail: None }
    }

    pub fn with_detail(code: ErrorCode, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: Some(detail.into()),
        }
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.detail {
            Some(detail) => write!(f, "{:?}: {detail}", self.code),
            None => write!(f, "{:?}", self.code),
        }
    }
}

impl std::error::Error for AppError {}

impl From<std::io::Error> for AppError {
    fn from(value: std::io::Error) -> Self {
        Self::with_detail(ErrorCode::Io, value.to_string())
    }
}
