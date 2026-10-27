//! 桌面深链（`ssh://`）与单实例唤起：Unix 域套接字实现的进程间转发。

mod service;
mod types;

#[cfg(test)]
mod tests;

pub use service::acquire_primary;
pub use types::{LaunchRequest, request_from_args};
