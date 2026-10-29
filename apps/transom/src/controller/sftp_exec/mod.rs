//! SFTP 操作的调度入口、任务执行体与结果回传助手。

mod dispatch;
mod ops;
mod progress;
mod remote_transfer;

pub(crate) use dispatch::{SftpDispatch, SpawnSftpOpParams, dispatch_sftp_op, spawn_sftp_op};
