//! 远程与本地系统资源统计数据解析器。

use crate::commands::SystemStats;

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
        },
        curr_cpu,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cpu_jiffies_and_percent() {
        let sample1 = "cpu  1000 200 300 8000 100 50 20 0\ncpu0 500 100 150 4000 50 25 10 0\n";
        let (idle1, total1) = parse_cpu_jiffies(sample1).expect("Failed to parse cpu1");
        assert_eq!(idle1, 8100);
        assert_eq!(total1, 9670);

        let sample2 = "cpu  1500 200 400 8500 100 50 20 0\n";
        let (idle2, total2) = parse_cpu_jiffies(sample2).expect("Failed to parse cpu2");
        assert_eq!(idle2, 8600);
        assert_eq!(total2, 10770);

        let percent = calculate_cpu_percent((idle1, total1), (idle2, total2));
        assert!(percent > 54.0 && percent < 55.0);
    }

    #[test]
    fn test_meminfo_and_uptime() {
        let mem = "MemTotal:       16384000 kB\nMemFree:         2000000 kB\nMemAvailable:   12000000 kB\n";
        let (used, total) = parse_meminfo(mem);
        assert_eq!(total, 16384000 * 1024);
        assert_eq!(used, 4384000 * 1024);

        let uptime = "123456.78 987654.32\n";
        assert_eq!(parse_uptime(uptime), 123456);

        let os = "NAME=\"Ubuntu\"\nPRETTY_NAME=\"Ubuntu 24.04.1 LTS\"\nVERSION_ID=\"24.04\"\n";
        assert_eq!(parse_os_name(os), "Ubuntu 24.04.1 LTS");
    }

    #[test]
    fn test_parse_uptime_in_combined_output() {
        let combined = "cpu  1000 200 300 8000 100 50 20 0\nMemTotal: 1000 kB\n185200.50 9999.00\nPRETTY_NAME=\"Debian\"\n";
        assert_eq!(parse_uptime(combined), 185200);

        let with_marker =
            "cpu  1000 200 300 8000 100 50 20 0\n__UPTIME__\n3600.25 1200.00\n__DF__\n";
        assert_eq!(parse_uptime(with_marker), 3600);
    }

    #[test]
    fn test_parse_disk() {
        let df_output = "Filesystem     1024-blocks      Used Available Capacity Mounted on\n/dev/sda1        104857600  45000000  59857600      43% /\n";
        let (used, total) = parse_disk(df_output);
        assert_eq!(total, 104857600 * 1024);
        assert_eq!(used, 45000000 * 1024);

        let combined = "MemTotal: 100 kB\n__DF__\n文件系统 1024的块 已用 可用 容量 挂载点\n/dev/nvme0n1p6 200000000 150000000 50000000 75% /\n__OS__\n";
        let (used2, total2) = parse_disk(combined);
        assert_eq!(total2, 200000000 * 1024);
        assert_eq!(used2, 150000000 * 1024);
    }
}
