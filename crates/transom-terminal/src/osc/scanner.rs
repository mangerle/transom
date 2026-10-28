//! PTY 字节流旁路 OSC 增量扫描器。

use super::types::{MAX_PAYLOAD_BYTES, OSC_INTRO, OscEvent};
use crate::sequence_scanner::SequenceScanner;

/// OSC 扫描器：序列边界复用通用骨架，载荷语义在本模块解析。
pub(crate) struct OscScanner {
    inner: SequenceScanner,
}

impl Default for OscScanner {
    fn default() -> Self {
        Self {
            inner: SequenceScanner::new(OSC_INTRO, MAX_PAYLOAD_BYTES),
        }
    }
}

impl OscScanner {
    /// 喂入一段字节流，返回其中识别到的事件。不消费输入，原样透传给内核解析器。
    #[cfg(test)]
    pub(crate) fn feed(&mut self, bytes: &[u8]) -> Vec<OscEvent> {
        self.feed_with_offsets(bytes)
            .into_iter()
            .map(|(_, event)| event)
            .collect()
    }

    /// 与 [`Self::feed`] 相同，但附带序列末字节在本段中的下标。
    pub(crate) fn feed_with_offsets(&mut self, bytes: &[u8]) -> Vec<(usize, OscEvent)> {
        self.inner.feed_with_offsets(bytes, emit)
    }
}

/// 解析一条完整 OSC 载荷。
fn emit(bytes: &[u8], offset: usize, events: &mut Vec<(usize, OscEvent)>) {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return;
    };
    let Some((code, params)) = text.split_once(';') else {
        return;
    };
    let parse = match code {
        "133" => parse_command_marker(params),
        "777" => parse_notify(params),
        _ => None,
    };
    if let Some(event) = parse {
        events.push((offset, event));
    }
}

/// OSC 133 的参数：iTerm2/WezTerm 系 shell 集成用它标记提示符与命令边界。
fn parse_command_marker(params: &str) -> Option<OscEvent> {
    let mut parts = params.split(';');
    let mark = parts.next()?;
    match mark {
        "A" => Some(OscEvent::PromptStart),
        "B" => Some(OscEvent::CommandStart),
        "D" => Some(OscEvent::CommandFinish {
            exit_code: parts.next().and_then(|value| value.parse::<i32>().ok()),
            duration_ms: parts.next().and_then(|value| value.parse::<u64>().ok()),
        }),
        "C" | "E" => Some(OscEvent::PreExec {
            command: parts
                .next()
                .map(str::to_string)
                .filter(|text| !text.is_empty()),
        }),
        _ => None,
    }
}

/// OSC 777：`notify;标题;正文` 或 `notify;title=..;body=..;icon=..`。
fn parse_notify(params: &str) -> Option<OscEvent> {
    let mut parts = params.split(';');
    if parts.next()? != "notify" {
        return None;
    }
    let mut title = String::new();
    let mut body = String::new();
    let mut positional: Vec<&str> = Vec::with_capacity(2);
    for part in parts.take(8) {
        match part.split_once('=') {
            Some(("title", value)) => title = value.to_string(),
            Some(("body", value)) | Some(("message", value)) => body = value.to_string(),
            Some((_, _)) => {}
            None if positional.len() < 2 => positional.push(part),
            None => {}
        }
    }
    if title.is_empty() {
        title = positional.first().copied().unwrap_or_default().to_string();
    }
    if body.is_empty() {
        body = positional.get(1).copied().unwrap_or_default().to_string();
    }
    if title.is_empty() && body.is_empty() {
        return None;
    }
    Some(OscEvent::Notify { title, body })
}
