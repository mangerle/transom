//! SFTP 客户端与传输操作子模块。
//!
//! 单文件传输（`transfer`）、悬停预览（`preview`）与跨远端互传（`remote_copy`）
//! 按关注点分文件：三者的失败语义、并发模型与数据落点完全不同，混在一个文件里
//! 会让“传输起点”“预览上限”“跨端属性保留”等无关约束互相纠缠。

pub mod check_file;
mod client;
pub(crate) mod dir;
pub(crate) mod hash;
mod ops;
mod preview;
mod remote_copy;
mod scan;
pub mod sudo_write;
pub(crate) mod sync;
pub(crate) mod text;
mod transfer;

#[cfg(test)]
mod tests;

pub use client::SftpClient;
pub(crate) use client::{TRANSFER_CHUNK, notify_shared_progress, sftp_error, sftp_io_error};
pub use transfer::TransferProgressResult;
