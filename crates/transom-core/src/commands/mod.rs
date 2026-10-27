//! 应用顶层命令与事件总线定义。
//!
//! 顶层总线划分为分领域子枚举，各个业务子系统可独立引用或处理其专属子枚举，
//! 降低跨 crate 编译依赖与巨型 match 分支。

mod domain_commands;
mod domain_events;

pub use domain_commands::{
    ConnectParams, ForwardCommand, HostCommand, SftpCommand, SyncCommand, TerminalCommand,
    ZmodemCommand,
};
pub use domain_events::{
    DeepLinkRequest, ForwardAppEvent, HostAppEvent, SftpAppEvent, SyncAppEvent, TerminalAppEvent,
    ZmodemAppEvent,
};

/// 应用顶层命令：按业务领域划分为子枚举。
pub enum AppCommand {
    Terminal(TerminalCommand),
    Sftp(SftpCommand),
    Forward(ForwardCommand),
    Host(HostCommand),
    Sync(SyncCommand),
    Zmodem(ZmodemCommand),
}

impl From<TerminalCommand> for AppCommand {
    fn from(cmd: TerminalCommand) -> Self {
        Self::Terminal(cmd)
    }
}

impl From<SftpCommand> for AppCommand {
    fn from(cmd: SftpCommand) -> Self {
        Self::Sftp(cmd)
    }
}

impl From<ForwardCommand> for AppCommand {
    fn from(cmd: ForwardCommand) -> Self {
        Self::Forward(cmd)
    }
}

impl From<HostCommand> for AppCommand {
    fn from(cmd: HostCommand) -> Self {
        Self::Host(cmd)
    }
}

impl From<SyncCommand> for AppCommand {
    fn from(cmd: SyncCommand) -> Self {
        Self::Sync(cmd)
    }
}

impl From<ZmodemCommand> for AppCommand {
    fn from(cmd: ZmodemCommand) -> Self {
        Self::Zmodem(cmd)
    }
}

/// 应用顶层事件：按业务领域划分为子枚举。
pub enum AppEvent {
    Terminal(TerminalAppEvent),
    Sftp(SftpAppEvent),
    Forward(ForwardAppEvent),
    Host(HostAppEvent),
    Sync(SyncAppEvent),
    Zmodem(ZmodemAppEvent),
    DeepLink { request: DeepLinkRequest },
}

impl From<TerminalAppEvent> for AppEvent {
    fn from(event: TerminalAppEvent) -> Self {
        Self::Terminal(event)
    }
}

impl From<SftpAppEvent> for AppEvent {
    fn from(event: SftpAppEvent) -> Self {
        Self::Sftp(event)
    }
}

impl From<ForwardAppEvent> for AppEvent {
    fn from(event: ForwardAppEvent) -> Self {
        Self::Forward(event)
    }
}

impl From<HostAppEvent> for AppEvent {
    fn from(event: HostAppEvent) -> Self {
        Self::Host(event)
    }
}

impl From<SyncAppEvent> for AppEvent {
    fn from(event: SyncAppEvent) -> Self {
        Self::Sync(event)
    }
}

impl From<ZmodemAppEvent> for AppEvent {
    fn from(event: ZmodemAppEvent) -> Self {
        Self::Zmodem(event)
    }
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct SystemStats {
    pub cpu_percent: f32,
    pub mem_used_bytes: u64,
    pub mem_total_bytes: u64,
    /// 根目录占用；多挂载点明细见 `filesystems`，该字段保留给状态栏。
    pub disk_used_bytes: u64,
    pub disk_total_bytes: u64,
    pub uptime_secs: u64,
    pub os_name: String,
    /// 每个逻辑核心的使用率；旧配置或首轮采样时为空。
    #[serde(default)]
    pub cpu_cores: Vec<crate::system_stats::CoreUsage>,
    /// 全部真实挂载点的占用明细。
    #[serde(default)]
    pub filesystems: Vec<crate::system_stats::FilesystemUsage>,
    /// 每块非回环网卡的本轮速率。
    #[serde(default)]
    pub interfaces: Vec<crate::system_stats::InterfaceRate>,
}
