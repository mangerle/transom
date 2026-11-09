//! 容器模块全局居中模态弹窗。
//!
//! 将危险操作确认、重命名、打标签、提交镜像、配置导出等从右侧详情页内嵌横条
//! 彻底移入组件库弹窗栈（dialog_host），统一呈现为标准居中模态对话框。

mod commit;
mod confirm;
mod export;
mod files;
mod forward;
mod network;
mod pods;
mod quadlet;
mod rename;
mod tag;
mod update;

pub(crate) use commit::render_container_commit_dialog_body;
pub(crate) use confirm::render_container_confirm_dialog_body;
pub(crate) use export::render_container_export_config_dialog_body;
pub(crate) use files::{
    render_container_file_new_dir_dialog_body, render_container_file_rename_dialog_body,
};
pub(crate) use forward::render_container_forward_dialog_body;
pub(crate) use network::render_container_network_connect_dialog_body;
pub(crate) use pods::render_container_pods_dialog_body;
pub(crate) use quadlet::render_container_quadlet_dialog_body;
pub(crate) use rename::render_container_rename_dialog_body;
pub(crate) use tag::render_container_tag_dialog_body;
pub(crate) use update::render_container_update_dialog_body;
