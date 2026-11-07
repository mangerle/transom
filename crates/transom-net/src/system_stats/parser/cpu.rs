//! CPU 时间片、每核占用与负载均值解析。

/// CPU 时间片细分快照（jiffies 累计值）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CpuTimesSnapshot {
    pub user: u64,
    pub nice: u64,
    pub system: u64,
    pub idle: u64,
    pub iowait: u64,
    pub irq: u64,
    pub softirq: u64,
    pub steal: u64,
    pub idle_all: u64,
    pub total: u64,
}

/// 从 /proc/stat 解析 CPU 详细时间片快照。
pub fn parse_cpu_times(content: &str) -> Option<CpuTimesSnapshot> {
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
                return Some(CpuTimesSnapshot {
                    user,
                    nice,
                    system,
                    idle,
                    iowait,
                    irq,
                    softirq,
                    steal,
                    idle_all,
                    total,
                });
            }
        }
    }
    None
}

/// 计算两次采样之间的 CPU 使用细分 (总占用%, iowait%, system%, user%)
pub fn calculate_cpu_breakdown(
    prev: CpuTimesSnapshot,
    curr: CpuTimesSnapshot,
) -> (f32, f32, f32, f32) {
    let total_delta = curr.total.saturating_sub(prev.total);
    if total_delta == 0 {
        return (0.0, 0.0, 0.0, 0.0);
    }
    let idle_delta = curr.idle_all.saturating_sub(prev.idle_all);
    let used_delta = total_delta.saturating_sub(idle_delta);
    let iowait_delta = curr.iowait.saturating_sub(prev.iowait);
    let system_delta = curr
        .system
        .saturating_sub(prev.system)
        .saturating_add(curr.irq.saturating_sub(prev.irq))
        .saturating_add(curr.softirq.saturating_sub(prev.softirq));
    let user_delta = curr
        .user
        .saturating_sub(prev.user)
        .saturating_add(curr.nice.saturating_sub(prev.nice));

    let pct = |d: u64| ((d as f64 / total_delta as f64) * 100.0) as f32;
    (
        pct(used_delta),
        pct(iowait_delta),
        pct(system_delta),
        pct(user_delta),
    )
}

/// 从 /proc/stat 解析 CPU 计数：(idle, total)
pub fn parse_cpu_jiffies(content: &str) -> Option<(u64, u64)> {
    parse_cpu_times(content).map(|s| (s.idle_all, s.total))
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

/// 解析系统平均负载 Load Average：(1m, 5m, 15m)
pub fn parse_loadavg(content: &str) -> (f32, f32, f32) {
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some((_, rest)) = trimmed.split_once("load average:") {
            let parts: Vec<&str> = rest.split(',').map(|s| s.trim()).collect();
            if parts.len() >= 3
                && let (Ok(l1), Ok(l2), Ok(l3)) = (
                    parts[0].parse::<f32>(),
                    parts[1].parse::<f32>(),
                    parts[2].parse::<f32>(),
                )
            {
                return (l1, l2, l3);
            }
        }
        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        if parts.len() >= 3
            && !trimmed.contains(':')
            && !trimmed.starts_with("__")
            && let (Ok(l1), Ok(l2), Ok(l3)) = (
                parts[0].parse::<f32>(),
                parts[1].parse::<f32>(),
                parts[2].parse::<f32>(),
            )
        {
            return (l1, l2, l3);
        }
    }
    (0.0, 0.0, 0.0)
}
