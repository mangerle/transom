//! 系统指标输出解析：CPU、内存、磁盘、网卡、进程与合并采样。

use std::time::{Duration, Instant};
use transom_core::{CoreUsage, SystemStats};

mod cpu;
mod disk;
mod memory;
mod network;
mod processes;
mod system;

pub use self::cpu::*;
pub use self::disk::*;
pub use self::memory::*;
pub use self::network::*;
pub use self::processes::*;
pub use self::system::*;

/// 跨采样轮次的累计状态：CPU 总量、每个核心与网卡累计字节。
#[derive(Debug, Default)]
pub struct StatsSamplerState {
    previous_cpu_times: Option<CpuTimesSnapshot>,
    previous_cores: Vec<(u64, u64)>,
    previous_net: Vec<(String, u64, u64)>,
    previous_at: Option<Instant>,
}

impl StatsSamplerState {
    /// 解析一轮组合输出并推进内部状态。
    pub fn ingest(&mut self, output: &str, now: Instant) -> SystemStats {
        let curr_cpu_times = parse_cpu_times(output);
        let (cpu_percent, cpu_iowait_percent, cpu_system_percent, cpu_user_percent) =
            match (self.previous_cpu_times, curr_cpu_times) {
                (Some(previous), Some(current)) => calculate_cpu_breakdown(previous, current),
                _ => (0.0, 0.0, 0.0, 0.0),
            };
        self.previous_cpu_times = curr_cpu_times;

        let cores = parse_cpu_cores(output);
        let cpu_cores = if self.previous_cores.len() == cores.len() {
            self.previous_cores
                .iter()
                .zip(&cores)
                .enumerate()
                .map(|(index, (previous, current))| CoreUsage {
                    index,
                    percent: calculate_cpu_percent(*previous, *current),
                })
                .collect()
        } else {
            Vec::new()
        };
        self.previous_cores = cores;

        let interfaces_snapshot = parse_net_dev_interfaces(output);
        let elapsed = self.previous_at.map_or(Duration::ZERO, |previous| {
            now.saturating_duration_since(previous)
        });
        let interfaces =
            calculate_interface_rates(&self.previous_net, &interfaces_snapshot, elapsed);
        if !interfaces_snapshot.is_empty() {
            self.previous_net = interfaces_snapshot;
        }
        self.previous_at = Some(now);

        let (mem_used_bytes, mem_total_bytes, swap_used_bytes, swap_total_bytes) =
            parse_meminfo_full(output);
        let (disk_used_bytes, disk_total_bytes) = parse_disk(output);
        let filesystems = parse_filesystems(output);
        let load_avg = parse_loadavg(output);
        let ports = parse_ports(output);
        let failed_services = parse_failed_services(output);
        let recent_oom = parse_oom_alert(output);

        SystemStats {
            cpu_percent,
            mem_used_bytes,
            mem_total_bytes,
            disk_used_bytes,
            disk_total_bytes,
            uptime_secs: parse_uptime(output),
            os_name: parse_os_name(output),
            load_avg,
            cpu_iowait_percent,
            cpu_system_percent,
            cpu_user_percent,
            swap_used_bytes,
            swap_total_bytes,
            cpu_cores,
            filesystems,
            interfaces,
            ports,
            failed_services,
            recent_oom,
            capabilities: transom_core::SystemStatsCapabilities {
                cpu_breakdown: true,
                linux_diagnostics: true,
            },
        }
    }
}
