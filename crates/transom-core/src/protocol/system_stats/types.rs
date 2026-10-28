//! 远程与本地系统资源统计数据模型。

use serde::{Deserialize, Serialize};

/// 单个逻辑核心的使用率。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct CoreUsage {
    pub index: usize,
    pub percent: f32,
}

/// 单个挂载点的文件系统占用。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct FilesystemUsage {
    pub mount: String,
    pub used_bytes: u64,
    pub total_bytes: u64,
}

/// 单块网卡的本轮上下行速率（字节/秒）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct InterfaceRate {
    pub name: String,
    pub rx_bytes_per_sec: u64,
    pub tx_bytes_per_sec: u64,
}

/// 进程列表排序方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ProcessSort {
    #[default]
    Cpu,
    Memory,
}

/// 远端进程快照的一行。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ProcessInfo {
    pub pid: u32,
    pub ppid: u32,
    pub user: String,
    pub cpu_percent: f32,
    pub mem_percent: f32,
    pub rss_bytes: u64,
    pub cmdline: String,
    /// 完整命令行参数与启动参数；若未采集到则为短名。
    #[serde(default)]
    pub args: Option<String>,
}

/// 监听端口明细。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PortListeningInfo {
    pub protocol: String,
    pub bind_addr: String,
    pub port: u16,
    pub pid: Option<u32>,
    pub process_name: Option<String>,
}

/// 异常崩溃的 Systemd 服务。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct FailedServiceInfo {
    pub unit: String,
    pub load: String,
    pub active: String,
    pub sub: String,
    pub description: String,
}

/// 内核 OOM Killer 触发警报。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct OomAlert {
    pub process_name: String,
    pub pid: u32,
    pub message: String,
}
