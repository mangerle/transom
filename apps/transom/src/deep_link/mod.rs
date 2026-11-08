//! 桌面深链（`ssh://`）与三端单实例转发。
//!
//! Unix 使用私有域套接字，Windows 使用当前用户私有命名管道；请求线协议与
//! 安全校验完全共用，平台模块只负责连接生命周期。

mod protocol;
mod types;

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(test)]
mod tests;

pub use types::{LaunchRequest, request_from_args};
#[cfg(unix)]
pub use unix::acquire_primary;
#[cfg(windows)]
pub use windows::acquire_primary;
