//! asciinema v2 格式终端会话录制与回放模块。
//!
//! 设计说明（Why）：
//! - 实现初衷：用户交互会话需要录制并一键回放执行。采用业界标准 asciinema v2
//!   格式（`.cast`），不仅保证录制序列可在本软件内精准重放，亦可在任何支持该标准的
//!   外部工具（如 `asciinema play`、Web 播放器等）中回放，杜绝私有格式造成的数据孤岛；
//! - 核心优势：
//!   1. 录制器挂载在 I/O 线程与输入管道，支持分别记录输出 (`"o"`) 与输入 (`"i"`)；
//!   2. 采用秒级浮点相对时间戳，高精度还原打字间隔与命令响应节奏；
//!   3. 流式 JSON Lines 追加写入，即使非正常断电或崩溃也不会损坏已有录制片段；
//! - 回放支持：
//!   解析 `.cast` 文件后按事件流提取输入/输出序列，支持按原速或倍速重放，向终端注入输入。

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
use transom_core::private_fs;

/// asciinema v2 头部定义。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CastHeader {
    pub version: u32,
    pub width: u16,
    pub height: u16,
    #[serde(default)]
    pub timestamp: Option<u64>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub env: Option<HashMap<String, String>>,
}

/// 录制事件类型：输出 ("o") 或 输入 ("i")。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CastEventType {
    Output,
    Input,
}

impl CastEventType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Output => "o",
            Self::Input => "i",
        }
    }
}

impl std::str::FromStr for CastEventType {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "o" => Ok(Self::Output),
            "i" => Ok(Self::Input),
            _ => Err(()),
        }
    }
}

/// 单条录制事件（相对开始时间的秒数、类型、文本内容）。
#[derive(Debug, Clone, PartialEq)]
pub struct CastEvent {
    pub time: f64,
    pub event_type: CastEventType,
    pub data: String,
}

/// 会话录制器：记录终端交互输入与输出。
pub struct SessionRecorder {
    path: PathBuf,
    writer: Option<BufWriter<File>>,
    start_time: Instant,
    active: Arc<AtomicBool>,
}

impl SessionRecorder {
    /// 创建并启动新录制。
    pub fn start(
        path: PathBuf,
        width: u16,
        height: u16,
        title: Option<String>,
    ) -> std::io::Result<Self> {
        if let Some(parent) = path.parent() {
            private_fs::ensure_private_dir(parent)?;
        }
        let file = private_fs::open_private_truncate(&path)?;
        let mut writer = BufWriter::new(file);

        let now_sec = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let mut env = HashMap::new();
        if let Ok(term) = std::env::var("TERM") {
            env.insert("TERM".to_string(), term);
        }

        let header = CastHeader {
            version: 2,
            width,
            height,
            timestamp: Some(now_sec),
            title,
            env: Some(env),
        };

        let header_json = serde_json::to_string(&header)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        writer.write_all(header_json.as_bytes())?;
        writer.write_all(b"\n")?;
        writer.flush()?;

        Ok(Self {
            path,
            writer: Some(writer),
            start_time: Instant::now(),
            active: Arc::new(AtomicBool::new(true)),
        })
    }

    /// 记录事件。
    pub fn record(&mut self, event_type: CastEventType, bytes: &[u8]) {
        if !self.active.load(Ordering::Relaxed) {
            return;
        }
        let Some(writer) = self.writer.as_mut() else {
            return;
        };

        let time = self.start_time.elapsed().as_secs_f64();
        let data = String::from_utf8_lossy(bytes).into_owned();

        // 序列化单条事件元组: [time, event_type, data]
        let record = (time, event_type.as_str(), data);
        if let Ok(json) = serde_json::to_string(&record) {
            let _ = writer.write_all(json.as_bytes());
            let _ = writer.write_all(b"\n");
        }
    }

    /// 停止录制并刷盘。
    pub fn stop(&mut self) {
        self.active.store(false, Ordering::Relaxed);
        if let Some(mut writer) = self.writer.take() {
            let _ = writer.flush();
        }
    }

    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::Relaxed)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for SessionRecorder {
    fn drop(&mut self) {
        self.stop();
    }
}

/// 录制控制器句柄（可在线程与异步任务间共享）。
#[derive(Clone)]
pub struct RecorderHandle {
    inner: Arc<Mutex<Option<SessionRecorder>>>,
}

impl Default for RecorderHandle {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(None)),
        }
    }
}

impl RecorderHandle {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_recording(&self) -> bool {
        self.inner.lock().as_ref().is_some_and(|r| r.is_active())
    }

    pub fn current_path(&self) -> Option<PathBuf> {
        self.inner.lock().as_ref().map(|r| r.path().to_path_buf())
    }

    pub fn start(
        &self,
        path: PathBuf,
        width: u16,
        height: u16,
        title: Option<String>,
    ) -> std::io::Result<()> {
        let recorder = SessionRecorder::start(path, width, height, title)?;
        *self.inner.lock() = Some(recorder);
        Ok(())
    }

    pub fn stop(&self) -> Option<PathBuf> {
        let mut guard = self.inner.lock();
        if let Some(mut recorder) = guard.take() {
            recorder.stop();
            Some(recorder.path().to_path_buf())
        } else {
            None
        }
    }

    pub fn record_output(&self, bytes: &[u8]) {
        if let Some(recorder) = self.inner.lock().as_mut() {
            recorder.record(CastEventType::Output, bytes);
        }
    }

    pub fn record_input(&self, bytes: &[u8]) {
        if let Some(recorder) = self.inner.lock().as_mut() {
            recorder.record(CastEventType::Input, bytes);
        }
    }
}

/// 解析 `.cast` 文件并提取事件流。
pub fn parse_cast_file(path: &Path) -> std::io::Result<(CastHeader, Vec<CastEvent>)> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut lines = reader.lines();

    let first_line = lines
        .next()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "文件为空"))??;
    let header: CastHeader = serde_json::from_str(&first_line)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

    let mut events = Vec::new();
    for line in lines {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // 解析 [time, type, data]
        if let Ok(parsed) = serde_json::from_str::<(f64, String, String)>(trimmed)
            && let Ok(event_type) = parsed.1.parse::<CastEventType>()
        {
            events.push(CastEvent {
                time: parsed.0,
                event_type,
                data: parsed.2,
            });
        }
    }

    Ok((header, events))
}

/// 从 asciinema 事件流中提取可执行的输入动作（包含相对前一输入的延时与输入文本）。
#[derive(Debug, Clone, PartialEq)]
pub struct ReplayInputStep {
    /// 相对上一步的延时秒数（最高限制为 2 秒，避免重放时长时间发呆）。
    pub delay_secs: f64,
    /// 输入数据。
    pub text: String,
}

/// 提取录制事件中的全部输入步。
pub fn extract_replay_inputs(events: &[CastEvent], max_delay_secs: f64) -> Vec<ReplayInputStep> {
    let mut steps = Vec::new();
    let mut last_time = 0.0;
    let mut is_first = true;

    for ev in events {
        if ev.event_type == CastEventType::Input {
            let delay = if is_first {
                0.0
            } else {
                (ev.time - last_time).clamp(0.0, max_delay_secs)
            };
            steps.push(ReplayInputStep {
                delay_secs: delay,
                text: ev.data.clone(),
            });
            last_time = ev.time;
            is_first = false;
        }
    }

    steps
}

#[cfg(test)]
mod tests {
    use super::{CastEventType, RecorderHandle, parse_cast_file};

    #[test]
    fn test_record_and_parse_cast() {
        let temp_dir = std::env::temp_dir().join("transom_record_test");
        let cast_file = temp_dir.join("test_session.cast");

        let handle = RecorderHandle::new();
        assert!(!handle.is_recording());

        handle
            .start(cast_file.clone(), 80, 24, Some("测试录制".to_string()))
            .expect("启动录制失败");
        assert!(handle.is_recording());

        handle.record_output(b"Welcome to Transom\r\n");
        handle.record_input(b"ls -la\n");
        handle.record_output(b"total 0\r\n");

        let saved_path = handle.stop();
        assert_eq!(saved_path, Some(cast_file.clone()));
        assert!(!handle.is_recording());

        let (header, events) = parse_cast_file(&cast_file).expect("解析录制文件失败");
        assert_eq!(header.version, 2);
        assert_eq!(header.width, 80);
        assert_eq!(header.height, 24);
        assert_eq!(header.title.as_deref(), Some("测试录制"));

        assert_eq!(events.len(), 3);
        assert_eq!(events[0].event_type, CastEventType::Output);
        assert_eq!(events[0].data, "Welcome to Transom\r\n");
        assert_eq!(events[1].event_type, CastEventType::Input);
        assert_eq!(events[1].data, "ls -la\n");
        assert_eq!(events[2].event_type, CastEventType::Output);
        assert_eq!(events[2].data, "total 0\r\n");

        let _ = std::fs::remove_file(cast_file);
        let _ = std::fs::remove_dir(temp_dir);
    }
}
