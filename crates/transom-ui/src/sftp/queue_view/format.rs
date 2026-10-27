//! SFTP 速率与预计剩余时间（ETA）格式化工具。
//!
//! 设计原理（Why）：
//! - 实现初衷：为传输进度条与队列项提供标准的字节速率转换与时间格式化；
//! - 核心优势：纯无副作用函数，易于复用与单元测试覆盖。

/// 将字节/秒速率格式化为可读字符串（B/s, KB/s, MB/s, GB/s）。
pub(crate) fn format_speed(bytes_per_sec: u64) -> String {
    if bytes_per_sec >= 1024 * 1024 * 1024 {
        format!(
            "{:.1} GB/s",
            bytes_per_sec as f64 / (1024.0 * 1024.0 * 1024.0)
        )
    } else if bytes_per_sec >= 1024 * 1024 {
        format!("{:.1} MB/s", bytes_per_sec as f64 / (1024.0 * 1024.0))
    } else if bytes_per_sec >= 1024 {
        format!("{:.1} KB/s", bytes_per_sec as f64 / 1024.0)
    } else {
        format!("{bytes_per_sec} B/s")
    }
}

/// 将剩余秒数格式化为可读的 ETA 文本（"剩余 mm:ss" 或 "剩余 hh:mm:ss"）。
pub(crate) fn format_eta(secs: u64) -> String {
    if secs >= 3600 {
        let hours = secs / 3600;
        let mins = (secs % 3600) / 60;
        let s = secs % 60;
        format!("剩余 {hours:02}:{mins:02}:{s:02}")
    } else {
        let mins = secs / 60;
        let s = secs % 60;
        format!("剩余 {mins:02}:{s:02}")
    }
}
