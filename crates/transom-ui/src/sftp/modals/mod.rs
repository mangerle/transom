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

pub(crate) use bridge::{
    close_sftp_chmod_modal, close_sftp_confirm_modal, close_sftp_filter_modal,
    close_sftp_input_modal, close_sftp_properties_modal, close_sftp_search_modal,
    close_sftp_sync_modal, open_sftp_chmod_modal, open_sftp_confirm_modal, open_sftp_filter_modal,
    open_sftp_properties_modal, open_sftp_search_modal, open_sftp_sync_modal,
    subscribe_sftp_input_modal as open_sftp_input_modal,
};
pub(crate) use input::SftpInputModalParams;
pub(crate) use state::{SftpModals, SftpPendingDelete};
