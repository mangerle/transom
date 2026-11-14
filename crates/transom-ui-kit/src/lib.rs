//! Transom 无状态表现层组件库：设计令牌、调色板、分割线、格式化与通用控件。
//!
//! 设计原理（Why）：将纯表现层与 `AppShell`、`AppState` 及后端服务物理隔离，
//! 从 Cargo 依赖图上确保组件层不可能反向耦合应用状态或存储。

pub mod divider;
pub mod format;
pub mod palette;
pub mod tokens;
pub mod widgets;
