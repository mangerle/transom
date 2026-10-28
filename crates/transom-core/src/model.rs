//! 领域模型与算法：会话、设置、目录、外观与终端配置。
//!
//! 设计原理（Why）：这些类型是"应用状态的形状"，被 UI、存储与网络层共同
//! 引用；按子域集中后，新增模型只在本目录内扩散，根模块保持稳定的契约面。
//! 外部调用方一律经本模块的规范路径引用；根模块只做常用类型的 re-export。

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
