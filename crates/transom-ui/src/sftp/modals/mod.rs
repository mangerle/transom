//! SFTP 权限、属性、过滤、同步、确认、输入与搜索弹窗。
//!
//! 每个弹窗都是独立的 GPUI 实体，通过事件向外壳上交意图；
//! 外壳桥接统一收敛在 [`bridge`] 模块中。

mod bridge;
mod chmod;
mod common;
mod confirm;
mod filter;
mod input;
mod properties;
mod state;

pub(crate) use chmod::SftpChmodModalParams;
pub(crate) use input::SftpInputModalParams;
pub(crate) use state::{SftpModals, SftpPendingDelete};
