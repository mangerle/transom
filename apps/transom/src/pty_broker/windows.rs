//! Windows 提权 PTY 代理的共享协议与子模块入口。

use std::path::PathBuf;
use transom_core::{AppError, ErrorCode, TerminalSize};

mod broker;
mod client;

pub(crate) use broker::run as run_broker;
pub(crate) use client::open_elevated_pty;

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct BrokerConfig {
    pub(super) shell: Option<String>,
    pub(super) shell_args: Vec<String>,
    pub(super) working_dir: Option<PathBuf>,
    pub(super) command: Option<Vec<String>>,
    pub(super) env: std::collections::BTreeMap<String, String>,
    pub(super) size: TerminalSize,
    pub(super) shell_integration: bool,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct BrokerReady {
    pub(super) integration: Option<transom_core::protocol::shell_integration::ShellKind>,
    pub(super) integration_disabled: bool,
}

pub(super) struct PipeNames {
    pub(super) config: String,
    pub(super) input: String,
    pub(super) output: String,
    pub(super) control: String,
}

impl PipeNames {
    pub(super) fn new(token: &str) -> Self {
        Self {
            config: format!(r"\\.\pipe\transom-pty-{token}-config"),
            input: format!(r"\\.\pipe\transom-pty-{token}-input"),
            output: format!(r"\\.\pipe\transom-pty-{token}-output"),
            control: format!(r"\\.\pipe\transom-pty-{token}-control"),
        }
    }
}

pub(super) fn io_error(operation: &str, error: std::io::Error) -> AppError {
    AppError::with_detail(ErrorCode::Io, format!("{operation}失败: {error}"))
}
