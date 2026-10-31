//! 宿主桌面集成：默认终端注册、桌面条目与启动器元数据。
//!
//! # 为什么独立成 crate
//! 这些能力改的是操作系统注册（XDG 配置、桌面数据库、图标主题），不是
//! 持久化数据；放在 `transom-store` 会让“存储”承担平台适配职责，也让
//! 未来 macOS / Windows 的注册逻辑无处安放。
//!
//! # 平台边界
//! - Linux：实现 freedesktop 的 Default Terminal Execution 规范；
//! - macOS / Windows：保留同名 API，当前明确返回“不支持”，后续可分别填入
//!   LaunchServices / 注册表实现，不影响上层调用点。
//!
//! 上层只依赖本 crate 的四个稳定入口：注册、撤销、查询、开关应用。

#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "linux")]
pub use linux::{
    apply_default_terminal, default_terminal_registered, desktop_entry_content, desktop_entry_id,
    preference_with_prepended, preference_without, register_default_terminal,
    unregister_default_terminal,
};

#[cfg(not(target_os = "linux"))]
mod unsupported;

#[cfg(not(target_os = "linux"))]
pub use unsupported::{
    apply_default_terminal, default_terminal_registered, register_default_terminal,
    unregister_default_terminal,
};
