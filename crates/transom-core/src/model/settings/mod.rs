//! 全局偏好与子系统设置。
//!
//! 设计原理（Why）：设置模型被 UI、存储、网络与终端组件广泛引用，涵盖外观、
//! 窗口状态、终端交互、SFTP、快捷指令、日志等级与语义块等领域。
//! 将原来 1270+ 行的大单体文件拆分为外观 (`appearance`)、窗口几何与布局 (`window`)、
//! 子系统设置 (`subsystems`) 及核心宿主设置 (`AppSettings`)，在遵循单一职责原则的同时，
//! 保持外部公开类型引用的完整平滑兼容。

mod appearance;
mod subsystems;
mod sync;
mod window;

#[cfg(test)]
mod tests;

pub use appearance::{AppearanceSettings, UiDensity, UiThemeMode};
pub use subsystems::{
    AuditSettings, BlockViewSettings, BlockViewStyle, HistorySettings, LogLevel, LogSettings,
    SftpSettings, ShortcutSettings, Snippet, SnippetSettings,
};
pub use sync::{GitSyncSettings, SyncEndpoint, SyncSettings};
pub use window::{
    BellStyle, PanelSide, SavedLayout, SavedTabGroup, SavedTabTiling, SidebarStartup, SplitAxis,
    StartupTabs, WindowBackgroundMode, WindowSettings,
};

use crate::model::layout::{ActivityBarLayout, StatusBarLayout};
use crate::model::terminal::{TerminalInteraction, TerminalProfile};
use serde::{Deserialize, Serialize};

/// 全局应用设置。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    #[serde(default = "default_language")]
    pub language: String,
    /// 界面外观（明暗、主题、密度）；旧配置缺少该字段时使用内置默认。
    #[serde(default)]
    pub appearance: AppearanceSettings,
    /// 本地终端启动的 shell；`None` 时跟随系统默认。
    ///
    /// 为什么是全局而不是会话级：本地终端从侧栏一键创建、没有会话表单，
    /// shell 是这台机器的属性而非某个会话的属性。
    #[serde(default)]
    pub local_shell: Option<String>,
    #[serde(default)]
    pub terminal_interaction: TerminalInteraction,
    #[serde(default)]
    pub terminal_profile: TerminalProfile,
    #[serde(default)]
    pub window: WindowSettings,
    #[serde(default)]
    pub sftp: SftpSettings,
    #[serde(default)]
    pub shortcuts: ShortcutSettings,
    #[serde(default)]
    pub snippets: SnippetSettings,
    /// 活动栏顺序、隐藏项与上下分区；旧配置缺少该字段时使用默认布局。
    #[serde(default)]
    pub activity_bar: ActivityBarLayout,
    /// 状态栏顺序与隐藏项；旧配置缺少该字段时使用默认布局。
    #[serde(default)]
    pub status_bar: StatusBarLayout,
    /// 云端同步端点与最近同步时间；旧配置缺少该字段时按未启用降级。
    #[serde(default)]
    pub sync: SyncSettings,
    /// ZMODEM 下载目录与自动接收开关；旧配置缺少该字段时使用默认。
    #[serde(default)]
    pub zmodem: crate::protocol::zmodem::ZmodemSettings,
    /// 会话输出审计落盘；旧配置缺少该字段时按关闭降级。
    #[serde(default)]
    pub audit: AuditSettings,
    /// 命令历史与建议；旧配置缺少该字段时按默认开启降级。
    #[serde(default)]
    pub history: HistorySettings,
    /// 运行日志级别；旧配置缺少该字段时按默认 `info` 降级。
    #[serde(default)]
    pub logs: LogSettings,
    /// 终端输出触发器与自动应答规则；旧配置缺少该字段时默认空。
    #[serde(default)]
    pub triggers: crate::model::trigger::TriggerSettings,
    /// 全局出站代理；会话未显式指定代理时生效，旧配置缺省为直连。
    #[serde(default)]
    pub proxy: crate::protocol::proxy::OutboundProxy,
    /// OSC 133 语义块视图偏好；旧配置缺省为启用卡片样式。
    #[serde(default)]
    pub block_view: BlockViewSettings,
    /// 允许用 SHA1（`ssh-rsa`）做用户公钥认证。
    ///
    /// 设计原理（Why）：OpenSSH 8.8+ 默认禁用 SHA1 用户签名，但工控设备、
    /// 交换机管理口等运维靶机常只支持 ssh-rsa；默认保持兼容（true）并仅在
    /// 实际使用时告警留痕，安全敏感的部署可改为 false 拒绝降级签名。
    /// 细粒度策略不进设置界面，由设置文件承载。
    #[serde(default = "default_allow_sha1_publickey_auth")]
    pub allow_sha1_publickey_auth: bool,
    /// 主机监控板块折叠/展开偏好设置；旧配置缺省时使用默认布局。
    #[serde(default)]
    pub monitor: MonitorSettings,
}

fn default_allow_sha1_publickey_auth() -> bool {
    true
}

fn default_language() -> String {
    "zh-CN".to_string()
}

fn default_true() -> bool {
    true
}

fn default_false() -> bool {
    false
}

/// 主机监控面板各板块默认折叠/展开偏好设置。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitorSettings {
    /// CPU 板块是否默认展开（默认 true）
    #[serde(default = "default_true")]
    pub cpu_expanded: bool,
    /// 内存板块是否默认展开（默认 true）
    #[serde(default = "default_true")]
    pub memory_expanded: bool,
    /// 磁盘板块是否默认展开（默认 false）
    #[serde(default = "default_false")]
    pub disk_expanded: bool,
    /// 网络板块是否默认展开（默认 true）
    #[serde(default = "default_true")]
    pub network_expanded: bool,
    /// 端口板块是否默认展开（默认 false）
    #[serde(default = "default_false")]
    pub ports_expanded: bool,
    /// 服务板块是否默认展开（默认 false）
    #[serde(default = "default_false")]
    pub services_expanded: bool,
    /// 进程板块是否默认展开（默认 true）
    #[serde(default = "default_true")]
    pub processes_expanded: bool,
}

impl Default for MonitorSettings {
    fn default() -> Self {
        Self {
            cpu_expanded: true,
            memory_expanded: true,
            disk_expanded: false,
            network_expanded: true,
            ports_expanded: false,
            services_expanded: false,
            processes_expanded: true,
        }
    }
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            language: default_language(),
            appearance: AppearanceSettings::default(),
            local_shell: None,
            terminal_interaction: TerminalInteraction::default(),
            terminal_profile: TerminalProfile::default(),
            window: WindowSettings::default(),
            sftp: SftpSettings::default(),
            shortcuts: ShortcutSettings::default(),
            snippets: SnippetSettings::default(),
            activity_bar: ActivityBarLayout::default(),
            status_bar: StatusBarLayout::default(),
            sync: SyncSettings::default(),
            zmodem: crate::protocol::zmodem::ZmodemSettings::default(),
            audit: AuditSettings::default(),
            history: HistorySettings::default(),
            logs: LogSettings::default(),
            triggers: crate::model::trigger::TriggerSettings::default(),
            proxy: crate::protocol::proxy::OutboundProxy::default(),
            block_view: BlockViewSettings::default(),
            allow_sha1_publickey_auth: default_allow_sha1_publickey_auth(),
            monitor: MonitorSettings::default(),
        }
    }
}
