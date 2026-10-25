//! 用户配置目录约定：全部落盘文件与资源目录的唯一来源。
//!
//! 为什么放 core：路径约定是跨 crate 契约（store 落盘、ui 读主题目录、应用
//! 组装都要用），store 侧重持久化实现，core 才是共同依赖的最底层。

use directories::{BaseDirs, UserDirs};
use std::path::PathBuf;

/// 应用桌面标识（reverse-DNS）。
///
/// WM_CLASS、.desktop 文件名与 `StartupWMClass` 必须逐字一致，否则任务栏
/// 分组与 `ssh://` 处理程序回指都会失效，所以只在这里定义一次。
pub const APP_ID: &str = "com.github.mangerle.transom";

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

/// 运行期私有目录（XDG runtime），存放只在进程存活期间有意义的文件。
///
/// 为什么与配置目录分开：套接字这类 IPC 端点不该被备份/同步波及，且
/// XDG_RUNTIME_DIR 是用户私有、会话结束即回收的 tmpfs；环境变量缺失时
/// （非桌面会话、容器）退回配置目录，功能不受影响。仅本模块内部使用，
/// 对外只暴露具体端点路径。
fn runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .map(|dir| dir.join("transom"))
        .unwrap_or_else(config_dir)
}

/// 单实例深链套接字路径；后启动的进程把 `ssh://` 地址写入这里转交给
/// 已运行实例。放在运行期目录并配合 0700 目录权限，天然限定同用户访问。
pub fn instance_socket_path() -> PathBuf {
    runtime_dir().join("transom.sock")
}

/// 用户自定义 UI 主题目录；放入 gpui-kit ThemeConfig 格式的 JSON 即可在设置
/// 页选用。内置主题也以此目录下的受管文件形式存在（应用启动时维护，顶层
/// author 为 "transom"），热重载时与用户主题走同一套目录重建语义；自定义
/// 主题请使用不同文件名，避免被应用维护的内置文件覆盖。
pub fn themes_dir() -> PathBuf {
    config_dir().join("themes")
}

/// ZMODEM 下载的默认目录：系统下载目录（XDG 配置感知），缺失时退回用户
/// 主目录，再退回当前目录。用户显式配置的目录优先级更高（见
/// `ZmodemSettings::resolved_download_dir`），这里只提供兜底值。
pub fn default_download_dir() -> PathBuf {
    UserDirs::new()
        .and_then(|dirs| dirs.download_dir().map(PathBuf::from))
        .or_else(|| {
            BaseDirs::new().map(|dirs| {
                let home = dirs.home_dir().to_path_buf();
                home.join("Downloads")
            })
        })
        .unwrap_or_else(|| PathBuf::from("."))
}
