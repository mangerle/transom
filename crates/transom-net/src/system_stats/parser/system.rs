//! 启动时长、发行版名称与整块输出的组合采样。

use super::{
    calculate_cpu_percent, parse_cpu_jiffies, parse_disk, parse_failed_services, parse_filesystems,
    parse_loadavg, parse_meminfo_full, parse_oom_alert, parse_ports,
};
use transom_core::SystemStats;

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

    let (mem_used_bytes, mem_total_bytes, swap_used_bytes, swap_total_bytes) =
        parse_meminfo_full(output);
    let (disk_used_bytes, disk_total_bytes) = parse_disk(output);
    let uptime_secs = parse_uptime(output);
    let os_name = parse_os_name(output);
    let load_avg = parse_loadavg(output);
    let ports = parse_ports(output);
    let failed_services = parse_failed_services(output);
    let recent_oom = parse_oom_alert(output);

    (
        SystemStats {
            cpu_percent,
            mem_used_bytes,
            mem_total_bytes,
            disk_used_bytes,
            disk_total_bytes,
            uptime_secs,
            os_name,
            load_avg,
            cpu_iowait_percent: 0.0,
            cpu_system_percent: 0.0,
            cpu_user_percent: 0.0,
            swap_used_bytes,
            swap_total_bytes,
            cpu_cores: Vec::new(),
            filesystems: parse_filesystems(output),
            interfaces: Vec::new(),
            ports,
            failed_services,
            recent_oom,
            capabilities: transom_core::SystemStatsCapabilities {
                cpu_breakdown: true,
                linux_diagnostics: true,
            },
        },
        curr_cpu,
    )
}
