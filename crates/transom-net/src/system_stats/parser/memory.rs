//! 内存与交换分区用量解析。

/// 从 /proc/meminfo 解析内存使用情况：(used_bytes, total_bytes)
pub fn parse_meminfo(content: &str) -> (u64, u64) {
    let (used, total, _, _) = parse_meminfo_full(content);
    (used, total)
}

/// 从 /proc/meminfo 解析物理内存与 Swap 分区：(used_bytes, total_bytes, swap_used_bytes, swap_total_bytes)
pub fn parse_meminfo_full(content: &str) -> (u64, u64, u64, u64) {
    let mut total_kb = 0u64;
    let mut avail_kb = 0u64;
    let mut swap_total_kb = 0u64;
    let mut swap_free_kb = 0u64;

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
            } else if key == "SwapTotal" {
                swap_total_kb = val;
            } else if key == "SwapFree" {
                swap_free_kb = val;
            }
        }
    }

    let total_bytes = total_kb * 1024;
    let used_bytes = total_kb.saturating_sub(avail_kb) * 1024;
    let swap_total_bytes = swap_total_kb * 1024;
    let swap_used_bytes = swap_total_kb.saturating_sub(swap_free_kb) * 1024;
    (used_bytes, total_bytes, swap_used_bytes, swap_total_bytes)
}
