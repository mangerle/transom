//! 应用内置审计纯文本日志查看器子模块。
//!
//! 状态、扫描与渲染归 [`AuditViewerView`]；外壳入口与事件消费在 `shell.rs`，
//! 渲染在 `render.rs`。本文件只保留模块声明与受控重导出。

mod entity;
mod render;
mod shell;
pub(crate) use shell::{on_audit_viewer_event, open_audit_viewer, open_audit_viewer_for_session, open_audit_viewer_for_tab};
mod types;

#[cfg(test)]
mod tests;

pub(crate) use entity::{AuditViewerEvent, AuditViewerView};
