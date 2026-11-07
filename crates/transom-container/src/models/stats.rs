//! 容器统计与进程列表的线格式结构体，以及采样差分的转换逻辑。

use super::null_as_empty;
use serde::Deserialize;
use std::collections::HashMap;
use transom_core::{
    ContainerBlockDeviceIo, ContainerInterfaceIo, ContainerProcess, ContainerStats,
};

/// `GET /containers/{id}/stats` 响应。
#[derive(Debug, Clone, Deserialize, Default)]
pub struct StatsResponse {
    #[serde(rename = "cpu_stats", default)]
    pub cpu_stats: CpuStats,
    #[serde(rename = "precpu_stats", default)]
    pub precpu_stats: CpuStats,
    #[serde(rename = "memory_stats", default)]
    pub memory_stats: MemoryStats,
    #[serde(rename = "networks", default)]
    pub networks: Option<HashMap<String, NetworkIo>>,
    #[serde(rename = "blkio_stats", default)]
    pub blkio_stats: Option<BlkioStats>,
    #[serde(rename = "pids_stats", default)]
    pub pids_stats: Option<PidsStats>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct CpuStats {
    #[serde(rename = "cpu_usage", default)]
    pub cpu_usage: CpuUsage,
    #[serde(rename = "system_cpu_usage", default)]
    pub system_cpu_usage: Option<u64>,
    #[serde(rename = "online_cpus", default)]
    pub online_cpus: Option<u32>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct CpuUsage {
    #[serde(rename = "total_usage", default)]
    pub total_usage: u64,
    #[serde(rename = "percpu_usage", default)]
    pub percpu_usage: Option<Vec<u64>>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct MemoryStats {
    #[serde(rename = "usage", default)]
    pub usage: u64,
    #[serde(rename = "limit", default)]
    pub limit: u64,
    /// 细分项（`rss`/`cache`/`inactive_file` 等）：统计页用它展示内存构成。
    #[serde(rename = "stats", default, deserialize_with = "null_as_empty")]
    pub stats: HashMap<String, u64>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct NetworkIo {
    #[serde(rename = "rx_bytes", default)]
    pub rx_bytes: u64,
    #[serde(rename = "tx_bytes", default)]
    pub tx_bytes: u64,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct BlkioStats {
    #[serde(rename = "io_service_bytes_recursive", default)]
    pub io_service_bytes_recursive: Option<Vec<BlkioEntry>>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct BlkioEntry {
    #[serde(rename = "op", default)]
    pub op: String,
    #[serde(rename = "value", default)]
    pub value: u64,
    #[serde(rename = "major", default)]
    pub major: Option<u16>,
    #[serde(rename = "minor", default)]
    pub minor: Option<u16>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct PidsStats {
    #[serde(rename = "current", default)]
    pub current: u64,
}

/// `GET /containers/{id}/top` 响应（`ps_args=aux`）。
#[derive(Debug, Clone, Deserialize, Default)]
pub struct TopResponse {
    #[serde(rename = "Titles", default, deserialize_with = "null_as_empty")]
    pub titles: Vec<String>,
    #[serde(rename = "Processes", default, deserialize_with = "null_as_empty")]
    pub processes: Vec<Vec<String>>,
}

impl TopResponse {
    /// 按标题定位列：不同镜像的 `ps` 列顺序可能不同，不能按下标硬取。
    pub fn into_processes(self) -> Vec<ContainerProcess> {
        let column = |name: &str| {
            self.titles
                .iter()
                .position(|title| title.eq_ignore_ascii_case(name))
        };
        let pid = column("PID");
        let user = column("USER");
        let cpu = column("%CPU");
        let mem = column("%MEM");
        let command = column("COMMAND")
            .or_else(|| column("COMMAND_AND_ARGS"))
            .or_else(|| column("ARGS"))
            .or_else(|| column("CMD"));
        self.processes
            .into_iter()
            .map(|row| {
                let cell = |index: Option<usize>| {
                    index
                        .and_then(|index| row.get(index))
                        .cloned()
                        .unwrap_or_default()
                };
                ContainerProcess {
                    pid: cell(pid),
                    user: cell(user),
                    cpu_percent: cell(cpu).parse().unwrap_or(0.0),
                    mem_percent: cell(mem).parse().unwrap_or(0.0),
                    command: command
                        .and_then(|index| row.get(index..).map(|tail| tail.join(" ")))
                        .unwrap_or_default(),
                }
            })
            .collect()
    }
}

impl StatsResponse {
    /// 计算界面展示的统计快照；`previous` 用于非流式响应的 CPU 差值兜底。
    pub fn to_stats(&self, previous: Option<&Self>) -> ContainerStats {
        let previous_cpu = if self.precpu_stats.cpu_usage.total_usage > 0 {
            &self.precpu_stats
        } else {
            previous
                .map(|sample| &sample.cpu_stats)
                .unwrap_or(&self.precpu_stats)
        };
        let cpu_delta = self
            .cpu_stats
            .cpu_usage
            .total_usage
            .saturating_sub(previous_cpu.cpu_usage.total_usage);
        let system_delta = self
            .cpu_stats
            .system_cpu_usage
            .unwrap_or(0)
            .saturating_sub(previous_cpu.system_cpu_usage.unwrap_or(0));
        let online_cpus = self
            .cpu_stats
            .online_cpus
            .map(u64::from)
            .or_else(|| {
                self.cpu_stats
                    .cpu_usage
                    .percpu_usage
                    .as_ref()
                    .map(|cores| cores.len() as u64)
            })
            .unwrap_or(1)
            .max(1);
        let cpu_percent = if system_delta > 0 {
            (cpu_delta as f64 / system_delta as f64 * online_cpus as f64 * 100.0) as f32
        } else {
            0.0
        };
        // 每核占用：用各核累计 jiffies 的差值除以单核系统时间增量。
        let mut cpu_cores = Vec::new();
        if system_delta > 0
            && let (Some(current), Some(previous)) = (
                self.cpu_stats.cpu_usage.percpu_usage.as_ref(),
                previous_cpu.cpu_usage.percpu_usage.as_ref(),
            )
            && current.len() == previous.len()
        {
            let per_core_system = system_delta as f64 / online_cpus as f64;
            cpu_cores = current
                .iter()
                .zip(previous)
                .map(|(now, before)| {
                    let delta = now.saturating_sub(*before) as f64;
                    (delta / per_core_system * 100.0).clamp(0.0, 100.0) as f32
                })
                .collect();
        }
        let memory_used = self.memory_stats.usage;
        let memory_limit = self.memory_stats.limit;
        let memory_percent = if memory_limit > 0 {
            (memory_used as f64 / memory_limit as f64 * 100.0) as f32
        } else {
            0.0
        };
        // cgroup v1 用 rss/cache，v2 用 anon/inactive_file，两套键都认。
        let memory_rss = self
            .memory_stats
            .stats
            .get("rss")
            .or_else(|| self.memory_stats.stats.get("anon"))
            .copied()
            .unwrap_or(0);
        let memory_cache = self
            .memory_stats
            .stats
            .get("cache")
            .or_else(|| self.memory_stats.stats.get("inactive_file"))
            .copied()
            .unwrap_or(0);
        let mut networks: Vec<ContainerInterfaceIo> = self
            .networks
            .as_ref()
            .map(|map| {
                map.iter()
                    .map(|(name, interface)| ContainerInterfaceIo {
                        name: name.clone(),
                        rx_bytes: interface.rx_bytes,
                        tx_bytes: interface.tx_bytes,
                    })
                    .collect()
            })
            .unwrap_or_default();
        networks.sort_by(|left, right| left.name.cmp(&right.name));
        let network_rx = networks
            .iter()
            .fold(0u64, |total, item| total.saturating_add(item.rx_bytes));
        let network_tx = networks
            .iter()
            .fold(0u64, |total, item| total.saturating_add(item.tx_bytes));
        let mut per_device: HashMap<(u16, u16), (u64, u64)> = HashMap::new();
        let (mut block_read, mut block_write) = (0u64, 0u64);
        if let Some(entries) = self
            .blkio_stats
            .as_ref()
            .and_then(|stats| stats.io_service_bytes_recursive.as_ref())
        {
            for entry in entries {
                match entry.op.as_str() {
                    "read" | "Read" => {
                        block_read = block_read.saturating_add(entry.value);
                        let slot = per_device
                            .entry((entry.major.unwrap_or(0), entry.minor.unwrap_or(0)))
                            .or_default();
                        slot.0 = slot.0.saturating_add(entry.value);
                    }
                    "write" | "Write" => {
                        block_write = block_write.saturating_add(entry.value);
                        let slot = per_device
                            .entry((entry.major.unwrap_or(0), entry.minor.unwrap_or(0)))
                            .or_default();
                        slot.1 = slot.1.saturating_add(entry.value);
                    }
                    _ => {}
                }
            }
        }
        let mut block_devices: Vec<ContainerBlockDeviceIo> = per_device
            .into_iter()
            .map(|((major, minor), (read, write))| ContainerBlockDeviceIo {
                device: format!("{major}:{minor}"),
                read_bytes: read,
                write_bytes: write,
            })
            .collect();
        block_devices.sort_by(|left, right| left.device.cmp(&right.device));
        ContainerStats {
            cpu_percent,
            cpu_count: online_cpus as u32,
            cpu_cores,
            host_load: None,
            memory_used,
            memory_limit,
            memory_percent,
            memory_rss,
            memory_cache,
            network_rx,
            network_tx,
            networks,
            block_read,
            block_write,
            block_devices,
            pids: self
                .pids_stats
                .as_ref()
                .map(|pids| pids.current)
                .unwrap_or(0),
        }
    }
}
