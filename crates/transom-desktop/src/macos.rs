//! macOS 桌面集成边界。
//!
//! macOS 没有 Linux 式系统默认终端。`ssh://` 关联由应用包的
//! `CFBundleURLTypes` 声明，LaunchServices 在安装/移动应用后维护注册。

use std::path::Path;
use transom_core::{AppError, ErrorCode};

/// macOS 不提供系统默认终端接管。
pub fn apply_default_terminal(_enabled: bool) -> Result<(), AppError> {
    Err(unsupported_default_terminal())
}

/// macOS 不提供系统默认终端注册。
pub fn register_default_terminal(_exe: &Path) -> Result<(), AppError> {
    Err(unsupported_default_terminal())
}

/// macOS 不提供系统默认终端撤销。
pub fn unregister_default_terminal() -> Result<(), AppError> {
    Err(unsupported_default_terminal())
}

/// macOS 默认终端概念不适用，恒为未注册。
pub fn default_terminal_registered(_exe: &Path) -> bool {
    false
}

fn unsupported_default_terminal() -> AppError {
    AppError::with_detail(
        ErrorCode::Configuration,
        "macOS 没有系统默认终端概念；ssh:// 关联由应用包 LaunchServices 元数据注册",
    )
}
