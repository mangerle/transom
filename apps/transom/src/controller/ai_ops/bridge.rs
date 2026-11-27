//! 控制平面与 `transom-ai` 工具执行器之间的桥接入口。

mod exec;
mod fs;
mod terminal;

pub(super) use super::runtime_bridge::HostRuntimeBridge;
pub(super) use exec::SshRemoteExecutor;
pub(super) use fs::SshRemoteFsBridge;
pub(super) use terminal::UiTerminalBridge;
