//! SFTP/FTP 操作的调度入口、任务执行体与结果回传助手。
//!
//! FTP 会话与 SFTP 共用 `SftpOperation` 契约与任务表，执行器按会话上下文
//! 里的客户端类型分流（见 `dispatch::dispatch_sftp_op`）。

mod dispatch;
mod ftp_exec;
mod ops;
mod progress;
mod remote_transfer;

pub(crate) use dispatch::{SftpDispatch, SpawnSftpOpParams, dispatch_sftp_op, spawn_sftp_op};
