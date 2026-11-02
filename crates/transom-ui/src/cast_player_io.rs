//! 播放器的字节通道与时长格式化辅助。

use std::io::{Cursor, Read};

/// 管道流式读取器：将 flume 接收端适配为标准 Read trait。
pub(super) struct ChannelReader {
    rx: flume::Receiver<Vec<u8>>,
    current: Cursor<Vec<u8>>,
}

impl ChannelReader {
    pub(super) fn new(rx: flume::Receiver<Vec<u8>>) -> Self {
        Self {
            rx,
            current: Cursor::new(Vec::new()),
        }
    }
}

/// 将秒数格式化为 `mm:ss` 或 `hh:mm:ss`。
pub(super) fn format_duration(seconds: f64) -> String {
    let total_secs = seconds.max(0.0).round() as u64;
    let hours = total_secs / 3600;
    let mins = (total_secs % 3600) / 60;
    let secs = total_secs % 60;
    if hours > 0 {
        format!("{hours:02}:{mins:02}:{secs:02}")
    } else {
        format!("{mins:02}:{secs:02}")
    }
}

impl Read for ChannelReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        loop {
            let n = self.current.read(buf)?;
            if n > 0 {
                return Ok(n);
            }
            match self.rx.recv() {
                Ok(chunk) => {
                    self.current = Cursor::new(chunk);
                }
                Err(_) => return Ok(0),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::format_duration;

    /// 秒数格式化为 `mm:ss`，超过一小时补出小时段；负数按 0 处理。
    #[test]
    fn formats_durations_across_boundaries() {
        assert_eq!(format_duration(0.0), "00:00");
        assert_eq!(format_duration(59.6), "01:00");
        assert_eq!(format_duration(61.0), "01:01");
        assert_eq!(format_duration(3600.0), "01:00:00");
        assert_eq!(format_duration(-5.0), "00:00");
    }
}
