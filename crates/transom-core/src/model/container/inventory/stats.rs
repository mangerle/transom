//! 容器统计采样、接口/设备 IO 与进程列表。

use serde::{Deserialize, Serialize};

/// 容器资源统计快照。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ContainerStats {
    /// CPU 占用百分比（0-100，按全部核心归一）。
    pub cpu_percent: f32,
    /// 逻辑核心数；界面用它显示「16 核」。
    #[serde(default)]
    pub cpu_count: u32,
    /// 每个逻辑核心的占用百分比；引擎不回报时为空。
    #[serde(default)]
    pub cpu_cores: Vec<f32>,
    /// 宿主机平均负载（1/5/15 分钟）。容器共享宿主内核，这不是容器自己的负载。
    #[serde(default)]
    pub host_load: Option<(f32, f32, f32)>,
    pub memory_used: u64,
    pub memory_limit: u64,
    /// 内存占用百分比（0-100）。
    pub memory_percent: f32,
    /// 常驻内存（RSS）；cgroup 未提供时为 0。
    #[serde(default)]
    pub memory_rss: u64,
    /// 页缓存：cgroup v1 取 `cache`，v2 取 `inactive_file`（Docker 即用后者）。
    #[serde(default)]
    pub memory_cache: u64,
    pub network_rx: u64,
    pub network_tx: u64,
    /// 每块网卡的累计收发：界面按相邻采样差值算速率。
    #[serde(default)]
    pub networks: Vec<ContainerInterfaceIo>,
    pub block_read: u64,
    pub block_write: u64,
    /// 按设备聚合的累计块 IO。
    #[serde(default)]
    pub block_devices: Vec<ContainerBlockDeviceIo>,
    /// 进程/线程数。
    pub pids: u64,
}

/// 单块网卡的累计收发字节。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ContainerInterfaceIo {
    pub name: String,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
}

/// 单个块设备的累计读写字节。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ContainerBlockDeviceIo {
    /// `major:minor`，例如 `8:0`。
    pub device: String,
    pub read_bytes: u64,
    pub write_bytes: u64,
}

/// 容器内的一个进程（来自 `GET /containers/{id}/top`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ContainerProcess {
    pub pid: String,
    pub user: String,
    /// CPU 占用百分比（`ps aux` 的 `%CPU`）；容器内 ps 不提供时为空。
    #[serde(default)]
    pub cpu_percent: f32,
    /// 内存占用百分比（`ps aux` 的 `%MEM`）。
    #[serde(default)]
    pub mem_percent: f32,
    pub command: String,
}
