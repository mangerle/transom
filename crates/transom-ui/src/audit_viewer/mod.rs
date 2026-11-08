//! 应用内置审计纯文本日志查看器子模块。

mod entity;
mod ops;
mod types;
mod view;

#[cfg(test)]
mod tests;

pub(crate) use entity::AuditViewerView;
