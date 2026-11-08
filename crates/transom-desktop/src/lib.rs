//! 宿主桌面集成：默认终端注册、桌面条目与启动器元数据。
//!
//! # 为什么独立成 crate
//! 这些能力改的是操作系统注册（XDG 配置、桌面数据库、图标主题），不是
//! 持久化数据；放在 `transom-store` 会让“存储”承担平台适配职责，也让
//! 未来 macOS / Windows 的注册逻辑无处安放。
//!
//! # 平台边界
//! - Linux：实现 freedesktop 的 Default Terminal Execution 规范；
//! - Windows：注册 `ssh://` 默认应用候选；系统默认终端仍由 Windows Console
//!   Handoff 管理，不伪造不完整的 COM 接管；
//! - macOS：URL Scheme 由应用包 `CFBundleURLTypes` 注册，默认终端概念不适用。
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

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub use windows::{
    apply_default_terminal, default_terminal_registered, register_default_terminal,
    unregister_default_terminal,
};

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::{
    apply_default_terminal, default_terminal_registered, register_default_terminal,
    unregister_default_terminal,
};

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
mod unsupported;

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
pub use unsupported::{
    apply_default_terminal, default_terminal_registered, register_default_terminal,
    unregister_default_terminal,
};

/// 当前平台的桌面集成能力。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DesktopIntegrationCapabilities {
    /// 是否存在可安全实现的系统默认终端接管机制。
    pub default_terminal: bool,
    /// 是否支持 `ssh://` URL Scheme 注册。
    pub uri_handler: bool,
}

/// 返回当前平台的桌面集成能力。
pub fn capabilities() -> DesktopIntegrationCapabilities {
    #[cfg(target_os = "linux")]
    {
        DesktopIntegrationCapabilities {
            default_terminal: true,
            uri_handler: true,
        }
    }
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    {
        DesktopIntegrationCapabilities {
            default_terminal: false,
            uri_handler: true,
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    {
        DesktopIntegrationCapabilities {
            default_terminal: false,
            uri_handler: false,
        }
    }
}

/// 确保 `ssh://` 处理程序已注册，但绝不改变系统默认应用。
///
/// Linux/macOS 由安装包元数据负责；Windows 写入每用户默认应用候选登记。
pub fn ensure_uri_handler(exe: &std::path::Path) -> Result<(), transom_core::AppError> {
    #[cfg(target_os = "windows")]
    {
        windows::ensure_uri_handler(exe)
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        let _ = exe;
        Ok(())
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    {
        let _ = exe;
        Err(transom_core::AppError::with_detail(
            transom_core::ErrorCode::Configuration,
            "当前平台不支持 ssh URL Scheme 注册",
        ))
    }
}
