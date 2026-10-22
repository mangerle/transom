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
    ForwardFailed,
    Io,
    SecretStoreUnavailable,
    Cancelled,
    Protocol,
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
