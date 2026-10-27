//! 状态栏数值格式化单元测试。

use super::{format_compact_size, format_rate};

#[test]
fn formats_rates_with_compact_units() {
    assert_eq!(format_rate(0), "0B/s");
    assert_eq!(format_rate(512), "512B/s");
    assert_eq!(format_rate(1536), "1.5K/s");
    assert_eq!(format_rate(1024 * 1024), "1.0M/s");
    assert_eq!(format_rate(3 * 1024 * 1024 * 1024), "3.0G/s");
    assert_eq!(format_rate(1024 * 1024 / 2), "512K/s");
}

#[test]
fn formats_sizes_with_one_decimal_below_hundred() {
    const KIB: u64 = 1024;
    const MIB: u64 = KIB * 1024;
    const GIB: u64 = MIB * 1024;
    const TIB: u64 = GIB * 1024;
    // 不足 1 KiB 仍是字节，不带小数。
    assert_eq!(format_compact_size(0), "0B");
    assert_eq!(format_compact_size(1023), "1023B");
    // 小于 100 保留一位小数：小内存机器不再被抹成整数。
    assert_eq!(format_compact_size(KIB), "1.0K");
    assert_eq!(format_compact_size(1536 * MIB), "1.5G");
    assert_eq!(format_compact_size(15 * GIB + GIB / 2), "15.5G");
    // 大于等于 100 取整，避免 `999.9G` 这类长串挤掉百分比。
    assert_eq!(format_compact_size(512 * GIB), "512G");
    assert_eq!(format_compact_size(1023 * GIB), "1023G");
    // TB 级磁盘自动进位，不会显示成 `1863G`。
    assert_eq!(format_compact_size(1536 * GIB), "1.5T");
    assert_eq!(format_compact_size(TIB), "1.0T");
}
