//! 其它平台的桌面集成占位实现。
//!
//! 仅供 Linux、Windows、macOS 之外的平台使用。三个主流平台各有独立实现；
//! 未支持平台仍能编译并拿到明确的“不支持”错误，而不是静默成功或直接 panic。

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
