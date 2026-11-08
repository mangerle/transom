//! 其它平台的桌面集成占位实现。
//!
//! macOS 与 Windows 没有 XDG 默认终端规范，后续应分别接 LaunchServices 与
//! 注册表；在实现完成前，上层仍能编译并拿到明确的“不支持”错误，而不是
//! 静默成功或直接 panic。

use std::path::Path;
use transom_core::{AppError, ErrorCode};

fn unsupported() -> AppError {
    AppError::with_detail(ErrorCode::Configuration, "当前平台暂不支持默认终端注册")
}

/// 应用默认终端开关；非 Linux 平台返回不支持。
///
/// # Errors
/// 当前平台没有默认终端注册实现时返回 [`ErrorCode::Configuration`]。
pub fn apply_default_terminal(_enabled: bool) -> Result<(), AppError> {
    Err(unsupported())
}

/// 注册当前可执行文件；非 Linux 平台返回不支持。
///
/// # Errors
/// 当前平台没有默认终端注册实现时返回 [`ErrorCode::Configuration`]。
pub fn register_default_terminal(_exe: &Path) -> Result<(), AppError> {
    Err(unsupported())
}

/// 撤销默认终端注册；非 Linux 平台返回不支持。
///
/// # Errors
/// 当前平台没有默认终端注册实现时返回 [`ErrorCode::Configuration`]。
pub fn unregister_default_terminal() -> Result<(), AppError> {
    Err(unsupported())
}

/// 查询是否已注册；非 Linux 平台恒为 `false`。
pub fn default_terminal_registered(_exe: &Path) -> bool {
    false
}
