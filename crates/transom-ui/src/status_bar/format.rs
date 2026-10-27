//! 状态栏数值格式化工具函数。
//!
//! 包含字节速率格式化与内存/磁盘容量紧凑格式化。

/// 将字节/秒格式化为紧凑单位，状态栏只保留数字与单字母单位。
///
/// 例如 `1.5K/s`、`123M/s`。完整口径由徽标 tooltip 说明；这样上下行两个速率
/// 不会因为 "KB/s" 这类长后缀把状态栏撑宽。
pub(crate) fn format_rate(bytes_per_sec: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;

    let value = bytes_per_sec as f64;
    if value < KB {
        return format!("{bytes_per_sec}B/s");
    }
    let (scaled, unit) = if value < MB {
        (value / KB, "K")
    } else if value < GB {
        (value / MB, "M")
    } else {
        (value / GB, "G")
    };
    if scaled < 100.0 {
        format!("{scaled:.1}{unit}/s")
    } else {
        format!("{scaled:.0}{unit}/s")
    }
}

/// 内存/磁盘体积的紧凑格式：按 1024 进位选单位，精度沿用 [`format_rate`] 的口径
/// （小于 100 保留一位小数，否则取整）。
///
/// 设计原理（Why）：原先直接写 `{:.0}G`，把 `3.7G` 显示成 `4G`，小内存机器与
/// 半满/将满的区分度全丢；同时 TB 级磁盘会显示成 `1863G` 这种四位长串。改为单位
/// 进位后字符串长度有界（最长 `1023G`），既能给出小数又不会撑爆数值槽。
pub(crate) fn format_compact_size(bytes: u64) -> String {
    const UNITS: [char; 5] = ['B', 'K', 'M', 'G', 'T'];
    let mut value = bytes as f64;
    let mut unit = 0usize;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        return format!("{bytes}B");
    }
    if value < 100.0 {
        format!("{value:.1}{}", UNITS[unit])
    } else {
        format!("{value:.0}{}", UNITS[unit])
    }
}
