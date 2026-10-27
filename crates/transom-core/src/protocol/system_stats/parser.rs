//! 系统指标输出解析函数（CPU、内存、磁盘、系统启动时间与网卡速率）。

use super::types::{CoreUsage, FilesystemUsage, InterfaceRate, ProcessInfo};
use crate::commands::SystemStats;
use std::time::{Duration, Instant};

/// 跨采样轮次的累计状态：CPU 总量、每个核心与网卡累计字节。
///
/// 速率与占比都必须用"两次采样之差"计算，状态因此不能散落在各调用点；
/// 首轮没有前值，相关字段为空，界面显示为零而不是编造数据。
#[derive(Debug, Default)]
pub struct StatsSamplerState {
    previous_cpu: Option<(u64, u64)>,
    previous_cores: Vec<(u64, u64)>,
    previous_net: Vec<(String, u64, u64)>,
    previous_at: Option<Instant>,
}

impl StatsSamplerState {
    /// 解析一轮组合输出并推进内部状态。
    pub fn ingest(&mut self, output: &str, now: Instant) -> SystemStats {
        let curr_cpu = parse_cpu_jiffies(output);
        let cpu_percent = match (self.previous_cpu, curr_cpu) {
            (Some(previous), Some(current)) => calculate_cpu_percent(previous, current),
            _ => 0.0,
        };
        self.previous_cpu = curr_cpu;

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

        let (mem_used_bytes, mem_total_bytes) = parse_meminfo(output);
        let (disk_used_bytes, disk_total_bytes) = parse_disk(output);
        let filesystems = parse_filesystems(output);
        SystemStats {
            cpu_percent,
            mem_used_bytes,
            mem_total_bytes,
            disk_used_bytes,
            disk_total_bytes,
            uptime_secs: parse_uptime(output),
            os_name: parse_os_name(output),
            cpu_cores,
            filesystems,
            interfaces,
        }
    }
}

/// 从 /proc/stat 解析 CPU 计数：(idle, total)
pub fn parse_cpu_jiffies(content: &str) -> Option<(u64, u64)> {
    for line in content.lines() {
        if line.starts_with("cpu ") {
            let parts: Vec<u64> = line
                .split_whitespace()
                .skip(1)
                .filter_map(|s| s.parse().ok())
                .collect();
            if parts.len() >= 4 {
                let user = parts[0];
                let nice = parts[1];
                let system = parts[2];
                let idle = parts[3];
                let iowait = parts.get(4).copied().unwrap_or(0);
                let irq = parts.get(5).copied().unwrap_or(0);
                let softirq = parts.get(6).copied().unwrap_or(0);
                let steal = parts.get(7).copied().unwrap_or(0);

                let idle_all = idle + iowait;
                let total = user + nice + system + idle_all + irq + softirq + steal;
                return Some((idle_all, total));
            }
        }
    }
    None
}

/// 计算两次采样之间的 CPU 使用百分比 (0.0 ~ 100.0)
pub fn calculate_cpu_percent(prev: (u64, u64), curr: (u64, u64)) -> f32 {
    let (prev_idle, prev_total) = prev;
    let (curr_idle, curr_total) = curr;

    let total_delta = curr_total.saturating_sub(prev_total);
    let idle_delta = curr_idle.saturating_sub(prev_idle);

    if total_delta == 0 {
        return 0.0;
    }

    let used_delta = total_delta.saturating_sub(idle_delta);
    ((used_delta as f64 / total_delta as f64) * 100.0) as f32
}

/// 从 /proc/meminfo 解析内存使用情况：(used_bytes, total_bytes)
pub fn parse_meminfo(content: &str) -> (u64, u64) {
    let mut total_kb = 0u64;
    let mut avail_kb = 0u64;

    for line in content.lines() {
        if let Some((k, v)) = line.split_once(':') {
            let key = k.trim();
            let val = v
                .split_whitespace()
                .next()
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(0);
            if key == "MemTotal" {
                total_kb = val;
            } else if key == "MemAvailable" {
                avail_kb = val;
            }
        }
    }

    let total_bytes = total_kb * 1024;
    let used_bytes = total_kb.saturating_sub(avail_kb) * 1024;
    (used_bytes, total_bytes)
}

/// 从 /proc/uptime 或复合输出中解析系统启动秒数
pub fn parse_uptime(content: &str) -> u64 {
    // 优先检查是否有明确的段落标记 __UPTIME__
    if let Some((_, rest)) = content.split_once("__UPTIME__") {
        for line in rest.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with("__") {
                continue;
            }
            if let Some(first) = trimmed.split_whitespace().next()
                && let Ok(val) = first.parse::<f64>()
            {
                return val as u64;
            }
        }
    }

    // 容错兜底：若没有段落标记，寻找整行只有 2 个浮点数的行（/proc/uptime 的标准特征）
    for line in content.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() == 2
            && let Ok(val) = parts[0].parse::<f64>()
            && parts[1].parse::<f64>().is_ok()
            && !line.contains(':')
            && !line.contains('=')
        {
            return val as u64;
        }
    }

    0
}

/// 从 df -P -k / 解析根目录磁盘使用情况：(used_bytes, total_bytes)
pub fn parse_disk(content: &str) -> (u64, u64) {
    let text = if let Some((_, rest)) = content.split_once("__DF__") {
        rest
    } else {
        content
    };

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("__") {
            break;
        }
        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        // df -P 格式下一行至少 6 项：Filesystem 1024-blocks Used Available Capacity Mounted on
        if parts.len() >= 6 && parts.last() == Some(&"/") {
            let total_idx = parts.len() - 5;
            let used_idx = parts.len() - 4;
            if let (Ok(total_kb), Ok(used_kb)) = (
                parts[total_idx].parse::<u64>(),
                parts[used_idx].parse::<u64>(),
            ) {
                return (used_kb * 1024, total_kb * 1024);
            }
        }
    }

    // 容错兜底：如果未显式挂载在 "/"，寻找带百分比的记录行
    for line in text.lines().rev() {
        let trimmed = line.trim();
        if trimmed.starts_with("__") {
            continue;
        }
        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        if parts.len() >= 6 && parts[parts.len() - 2].ends_with('%') {
            let total_idx = parts.len() - 5;
            let used_idx = parts.len() - 4;
            if let (Ok(total_kb), Ok(used_kb)) = (
                parts[total_idx].parse::<u64>(),
                parts[used_idx].parse::<u64>(),
            ) {
                return (used_kb * 1024, total_kb * 1024);
            }
        }
    }

    (0, 0)
}

/// 从 /etc/os-release 或输出中提取简短 OS 标识
pub fn parse_os_name(content: &str) -> String {
    for line in content.lines() {
        if line.starts_with("PRETTY_NAME=") {
            let name = line.trim_start_matches("PRETTY_NAME=").trim_matches('"');
            if !name.is_empty() {
                return name.to_string();
            }
        }
    }
    "Linux".to_string()
}

/// 从 /proc/stat 解析每个逻辑核心的 (idle, total)，顺序即 cpu0..cpuN。
pub fn parse_cpu_cores(content: &str) -> Vec<(u64, u64)> {
    let mut cores = Vec::with_capacity(8);
    for line in content.lines() {
        let Some(rest) = line.strip_prefix("cpu") else {
            continue;
        };
        // 汇总行是 `cpu `（后接空格），只有 `cpu0`/`cpu1` 这类核心行才算。
        if !rest.starts_with(|c: char| c.is_ascii_digit()) {
            continue;
        }
        let mut fields = rest.split_whitespace();
        let Some(name) = fields.next() else {
            continue;
        };
        if !name.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let parts: Vec<u64> = fields.filter_map(|value| value.parse().ok()).collect();
        if parts.len() < 4 {
            continue;
        }
        let idle_all = parts[3] + parts.get(4).copied().unwrap_or(0);
        let total = parts.iter().copied().sum::<u64>();
        cores.push((idle_all, total));
    }
    cores
}

/// 解析 `df -P -k` 的 `__DF__` 段，返回所有真实挂载点（最多 12 个）。
///
/// 只保留以 `/` 开头的挂载点：`df` 也可能列出伪文件系统，界面上的磁盘
/// 看板只关心真实路径；同一挂载点重复出现时保留首行。
pub fn parse_filesystems(content: &str) -> Vec<FilesystemUsage> {
    let text = if let Some((_, rest)) = content.split_once("__DF__") {
        rest
    } else {
        content
    };
    let mut filesystems: Vec<FilesystemUsage> = Vec::with_capacity(8);
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("__") {
            if trimmed.starts_with("__") && !filesystems.is_empty() {
                break;
            }
            continue;
        }
        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        // Filesystem 1024-blocks Used Available Capacity Mounted-on
        if parts.len() < 6 {
            continue;
        }
        let Some(mount) = parts.last() else {
            continue;
        };
        if !mount.starts_with('/') {
            continue;
        }
        let total_idx = parts.len() - 5;
        let used_idx = parts.len() - 4;
        let (Ok(total_kb), Ok(used_kb)) = (
            parts[total_idx].parse::<u64>(),
            parts[used_idx].parse::<u64>(),
        ) else {
            continue;
        };
        if total_kb == 0 {
            continue;
        }
        if filesystems.iter().any(|entry| entry.mount == *mount) {
            continue;
        }
        filesystems.push(FilesystemUsage {
            mount: (*mount).to_string(),
            used_bytes: used_kb.saturating_mul(1024),
            total_bytes: total_kb.saturating_mul(1024),
        });
        if filesystems.len() >= 12 {
            break;
        }
    }
    filesystems
}

/// 解析 `/proc/net/dev` 的 `__NET__` 段，返回非回环网卡的累计字节。
pub fn parse_net_dev_interfaces(content: &str) -> Vec<(String, u64, u64)> {
    let text = if let Some((_, rest)) = content.split_once("__NET__") {
        rest
    } else {
        content
    };
    let mut interfaces = Vec::with_capacity(4);
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("__") || trimmed.starts_with("Inter-") {
            if trimmed.starts_with("__") && !interfaces.is_empty() {
                break;
            }
            continue;
        }
        let Some((name, values)) = trimmed.split_once(':') else {
            continue;
        };
        let name = name.trim();
        if name.is_empty() || name == "lo" {
            continue;
        }
        let mut fields = values.split_whitespace();
        let Some(receive) = fields.next().and_then(|value| value.parse::<u64>().ok()) else {
            continue;
        };
        let Some(transmit) = fields.nth(7).and_then(|value| value.parse::<u64>().ok()) else {
            continue;
        };
        interfaces.push((name.to_string(), receive, transmit));
    }
    interfaces
}

/// 按两次累计快照计算每块网卡的速率。
///
/// 新出现的网卡没有前值，本轮速率记 0；消失的网卡不输出。
pub fn calculate_interface_rates(
    previous: &[(String, u64, u64)],
    current: &[(String, u64, u64)],
    elapsed: Duration,
) -> Vec<InterfaceRate> {
    let seconds = elapsed.as_secs_f64();
    if seconds <= 0.0 {
        return Vec::new();
    }
    let mut rates = Vec::with_capacity(current.len());
    for (name, rx, tx) in current {
        let previous_entry = previous.iter().find(|(previous, _, _)| previous == name);
        let (rx_before, tx_before) = previous_entry.map_or((*rx, *tx), |(_, rx, tx)| (*rx, *tx));
        let rate = |delta: u64| (delta as f64 / seconds).round() as u64;
        rates.push(InterfaceRate {
            name: name.clone(),
            rx_bytes_per_sec: rate(rx.saturating_sub(rx_before)),
            tx_bytes_per_sec: rate(tx.saturating_sub(tx_before)),
        });
    }
    rates
}

/// 解析 `ps -eo pid,ppid,user,%cpu,%mem,rss,comm` 输出。
///
/// 解析失败的行直接跳过：进程在采样瞬间退出很常见，不能因为一行坏数据
/// 让整个进程列表消失。
pub fn parse_processes(content: &str) -> Vec<ProcessInfo> {
    let mut processes = Vec::with_capacity(50);
    for line in content.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 7 {
            continue;
        }
        let Ok(pid) = parts[0].parse::<u32>() else {
            continue;
        };
        let Ok(ppid) = parts[1].parse::<u32>() else {
            continue;
        };
        let Ok(cpu_percent) = parts[3].parse::<f32>() else {
            continue;
        };
        let Ok(mem_percent) = parts[4].parse::<f32>() else {
            continue;
        };
        let Ok(rss_kb) = parts[5].parse::<u64>() else {
            continue;
        };
        processes.push(ProcessInfo {
            pid,
            ppid,
            user: parts[2].to_string(),
            cpu_percent,
            mem_percent,
            rss_bytes: rss_kb.saturating_mul(1024),
            cmdline: parts[6..].join(" "),
        });
    }
    processes
}

/// 将整体文本块（包含了 stat, meminfo, uptime, df, os-release）组合解析为 SystemStats
pub fn parse_combined_stats(
    output: &str,
    prev_cpu: Option<(u64, u64)>,
) -> (SystemStats, Option<(u64, u64)>) {
    let curr_cpu = parse_cpu_jiffies(output);
    let cpu_percent = match (prev_cpu, curr_cpu) {
        (Some(p), Some(c)) => calculate_cpu_percent(p, c),
        _ => 0.0,
    };

    let (mem_used_bytes, mem_total_bytes) = parse_meminfo(output);
    let (disk_used_bytes, disk_total_bytes) = parse_disk(output);
    let uptime_secs = parse_uptime(output);
    let os_name = parse_os_name(output);

    (
        SystemStats {
            cpu_percent,
            mem_used_bytes,
            mem_total_bytes,
            disk_used_bytes,
            disk_total_bytes,
            uptime_secs,
            os_name,
            cpu_cores: Vec::new(),
            filesystems: parse_filesystems(output),
            interfaces: Vec::new(),
        },
        curr_cpu,
    )
}
