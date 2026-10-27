//! 主机监控看板与进程管理器：Agentless，数据来自现有 SSH 采样与按需 exec。

mod ops;
mod types;
mod view;

pub(crate) use types::MonitorState;
