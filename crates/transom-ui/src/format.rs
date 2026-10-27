//! 通用数值、数据容量与速率格式化工具。
//!
//! 设计原理（Why）：
//! - 实现初衷：文件容量在 SFTP 列表、浮层、编辑器、ZMODEM 传输与系统监控中均需展示；
//!   此前各模块自行定义私有 `format_size`，存在单位字母规范不一、空格格式分裂与重复实现等坏味道；
//! - 核心优势：集中提供标准容量格式（带空格与全称单位，如 `1.5 MB`）、紧凑容量格式（如 `1.5M`）
//!   与紧凑速率格式（如 `1.5K/s`），保持全工程交互一致性；
//! - 代价与局限：仅覆盖常规二进制千（1024 进位）换算，不提供十进制千（1000 进位）选项。

/// 标准文件容量格式化：按 1024 进位，带空格与标准单位，保留一位小数。
///
/// 格式示例：`0 B`、`512 B`、`1.5 KB`、`10.2 MB`、`3.0 GB`。
pub(crate) fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0usize;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// 紧凑容量格式化：按 1024 进位选单位，小于 100 保留一位小数，否则取整。
///
/// 专用于状态栏、紧凑监控卡片等高密度显示区域，防止长串溢出。
/// 格式示例：`0B`、`512B`、`1.5K`、`120K`、`3.5G`。
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

/// 将字节/秒格式化为紧凑速率，只保留数字与单字母单位。
///
/// 格式示例：`0B/s`、`512B/s`、`1.5K/s`、`123M/s`。
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_size() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(1024), "1.0 KB");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_size(1024 * 1024), "1.0 MB");
        assert_eq!(format_size(5 * 1024 * 1024 * 1024), "5.0 GB");
    }

    #[test]
    fn test_format_compact_size() {
        assert_eq!(format_compact_size(0), "0B");
        assert_eq!(format_compact_size(512), "512B");
        assert_eq!(format_compact_size(1536), "1.5K");
        assert_eq!(format_compact_size(1024 * 1024), "1.0M");
        assert_eq!(format_compact_size(150 * 1024 * 1024), "150M");
    }

    #[test]
    fn test_format_rate() {
        assert_eq!(format_rate(0), "0B/s");
        assert_eq!(format_rate(512), "512B/s");
        assert_eq!(format_rate(1536), "1.5K/s");
        assert_eq!(format_rate(1024 * 1024), "1.0M/s");
        assert_eq!(format_rate(3 * 1024 * 1024 * 1024), "3.0G/s");
    }
}
