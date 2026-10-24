//! 用户配置目录约定：全部落盘文件与资源目录的唯一来源。
//!
//! 为什么放 core：路径约定是跨 crate 契约（store 落盘、ui 读主题目录、应用
//! 组装都要用），store 侧重持久化实现，core 才是共同依赖的最底层。

use directories::BaseDirs;
use std::path::PathBuf;

/// 应用配置根目录；无法定位用户目录时退回当前目录下的 `.transom`。
pub fn config_dir() -> PathBuf {
    BaseDirs::new()
        .map(|dirs| dirs.config_dir().join("transom"))
        .unwrap_or_else(|| PathBuf::from(".transom"))
}

pub fn known_hosts_path() -> PathBuf {
    config_dir().join("known_hosts.json")
}

pub fn sessions_path() -> PathBuf {
    config_dir().join("sessions.json")
}

pub fn settings_path() -> PathBuf {
    config_dir().join("settings.json")
}

/// 用户自定义 UI 主题目录；放入 gpui-kit ThemeConfig 格式的 JSON 即可在设置
/// 页选用。已知局限：主题注册表对重复名静默跳过，与内置主题（如 "Transom
/// Dark"）同名的用户主题不会生效，自定义主题请使用不同名称。
pub fn themes_dir() -> PathBuf {
    config_dir().join("themes")
}
