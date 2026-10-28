//! 领域模型与算法：会话、设置、目录、外观与终端配置。
//!
//! 设计原理（Why）：这些类型是"应用状态的形状"，被 UI、存储与网络层共同
//! 引用；按子域集中后，新增模型只在本目录内扩散，根模块保持稳定的契约面。
//! 历史路径（如 `transom_core::session`）由 lib.rs 的别名 re-export 保持
//! 可用，调用方不需要跟随目录调整。

pub mod catalog;
pub mod color;
pub mod forward;
pub mod history;
pub mod host;
pub mod identity;
pub mod known_hosts;
pub mod layout;
pub mod mac;
pub mod serial;
pub mod session;
pub mod session_appearance;
pub mod settings;
pub mod terminal;
pub mod theme;
pub mod trigger;
