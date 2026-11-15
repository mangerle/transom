//! SFTP 内置文本编辑器、权限、属性、过滤、同步与搜索弹窗。
//!
//! 状态与每类弹窗的视图分文件维护；共用入口继续由本模块重导出，
//! 外部调用点不需要感知内部拆分。

mod chmod;
mod common;
mod confirm;
mod filter;
mod input;
mod properties;
mod state;

pub(crate) use chmod::render_sftp_chmod_body;
pub(crate) use confirm::render_sftp_confirm_body;
pub(crate) use filter::render_sftp_filter_body;
pub(crate) use input::render_sftp_input_body;
pub(crate) use properties::render_sftp_properties_body;
pub(crate) use state::*;
