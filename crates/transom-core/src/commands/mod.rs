//! 应用顶层命令与事件总线定义。
//!
//! 顶层总线划分为分领域子枚举，各个业务子系统可独立引用或处理其专属子枚举，
//! 降低跨 crate 编译依赖与巨型 match 分支。

mod domain_commands;
mod domain_events;
mod git;

pub use domain_commands::{
    ConnectParams, ContainerCommand, ForwardCommand, HostCommand, SftpCommand, SyncCommand,
    TerminalCommand, ZmodemCommand,
};
pub use domain_events::{
    ContainerAppEvent, DeepLinkRequest, ForwardAppEvent, HostAppEvent, SftpAppEvent, SyncAppEvent,
    TerminalAppEvent, ZmodemAppEvent,
};
pub use git::{GitAppEvent, GitCommand, GitShelfOperation};

/// 应用顶层命令：按业务领域划分为子枚举。
pub enum AppCommand {
    Terminal(TerminalCommand),
    Sftp(SftpCommand),
    Forward(ForwardCommand),
    Host(HostCommand),
    Sync(Box<SyncCommand>),
    Zmodem(ZmodemCommand),
    /// 容器命令体量明显大于其它领域，装箱避免整个 AppCommand 被撑大。
    Container(Box<ContainerCommand>),
    Git(GitCommand),
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
        Self::Sync(Box::new(cmd))
    }
}

impl From<ZmodemCommand> for AppCommand {
    fn from(cmd: ZmodemCommand) -> Self {
        Self::Zmodem(cmd)
    }
}

impl From<ContainerCommand> for AppCommand {
    fn from(cmd: ContainerCommand) -> Self {
        Self::Container(Box::new(cmd))
    }
}

impl From<GitCommand> for AppCommand {
    fn from(cmd: GitCommand) -> Self {
        Self::Git(cmd)
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
    Container(ContainerAppEvent),
    Git(GitAppEvent),
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

impl From<ContainerAppEvent> for AppEvent {
    fn from(event: ContainerAppEvent) -> Self {
        Self::Container(event)
    }
}

impl From<GitAppEvent> for AppEvent {
    fn from(event: GitAppEvent) -> Self {
        Self::Git(event)
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
    /// 系统平均负载：(1分钟, 5分钟, 15分钟)。
    #[serde(default)]
    pub load_avg: (f32, f32, f32),
    /// CPU 等待 I/O 时间百分比。
    #[serde(default)]
    pub cpu_iowait_percent: f32,
    /// CPU 系统态占用百分比。
    #[serde(default)]
    pub cpu_system_percent: f32,
    /// CPU 用户态占用百分比。
    #[serde(default)]
    pub cpu_user_percent: f32,
    /// 交换分区已用字节。
    #[serde(default)]
    pub swap_used_bytes: u64,
    /// 交换分区总字节。
    #[serde(default)]
    pub swap_total_bytes: u64,
    /// 每个逻辑核心的使用率；旧配置或首轮采样时为空。
    #[serde(default)]
    pub cpu_cores: Vec<crate::protocol::system_stats::CoreUsage>,
    /// 全部真实挂载点的占用明细。
    #[serde(default)]
    pub filesystems: Vec<crate::protocol::system_stats::FilesystemUsage>,
    /// 每块非回环网卡的本轮速率。
    #[serde(default)]
    pub interfaces: Vec<crate::protocol::system_stats::InterfaceRate>,
    /// 监听端口明细。
    #[serde(default)]
    pub ports: Vec<crate::protocol::system_stats::PortListeningInfo>,
    /// 异常崩溃的 Systemd 服务。
    #[serde(default)]
    pub failed_services: Vec<crate::protocol::system_stats::FailedServiceInfo>,
    /// 近期内核 OOM 警报。
    #[serde(default)]
    pub recent_oom: Option<crate::protocol::system_stats::OomAlert>,
    /// 当前数据源实际支持的能力；缺失能力不得用零值伪装成真实数据。
    #[serde(default)]
    pub capabilities: SystemStatsCapabilities,
}

/// 系统统计的能力边界。
///
/// Linux `/proc` 与 systemd 诊断能提供完整信息，跨平台宿主采样器则只能
/// 提供通用资源指标。显式建模后，界面可以隐藏不存在的明细，而不是把
/// “未采集”显示成 0 或“无数据”。
#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct SystemStatsCapabilities {
    /// 是否提供用户态、系统态与 I/O 等待拆分。
    pub cpu_breakdown: bool,
    /// 是否提供监听端口、systemd 失败服务与内核 OOM 诊断。
    pub linux_diagnostics: bool,
}
