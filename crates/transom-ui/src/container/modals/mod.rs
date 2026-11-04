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
mod rename;
mod tag;
mod update;
