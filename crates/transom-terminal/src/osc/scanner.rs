//! PTY 字节流旁路 OSC 增量扫描器。

use super::types::{ESCAPE, MAX_PAYLOAD_BYTES, OSC_INTRO, OscEvent, TERMINATOR_BEL, TERMINATOR_ST};

/// 增量扫描状态。
#[derive(Debug, Default)]
pub(crate) struct OscScanner {
    /// 上一个字节是 `ESC`，正在确认它后面是不是 `]`。
    awaiting_intro: bool,
    /// 正在收集 OSC 载荷。
    in_payload: bool,
    /// 载荷里已见 `ESC`，下一个字节是 `\` 才算 ST 结束。
    awaiting_st: bool,
    /// 载荷已超长，丢弃内容直到序列结束。
    overflow: bool,
    payload: Vec<u8>,
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
        let mut events = Vec::new();
        for (offset, &byte) in bytes.iter().enumerate() {
            self.push(byte, offset, &mut events);
        }
        events
    }

    fn push(&mut self, byte: u8, offset: usize, events: &mut Vec<(usize, OscEvent)>) {
        if !self.in_payload {
            self.push_idle(byte);
            return;
        }
        if self.awaiting_st {
            self.awaiting_st = false;
            if byte == TERMINATOR_ST || byte == TERMINATOR_BEL {
                self.close(offset, events);
            } else {
                self.collect(ESCAPE, events);
                self.collect(byte, events);
            }
            return;
        }
        match byte {
            TERMINATOR_BEL => self.close(offset, events),
            ESCAPE => self.awaiting_st = true,
            other => self.collect(other, events),
        }
    }

    fn push_idle(&mut self, byte: u8) {
        if self.awaiting_intro {
            self.awaiting_intro = false;
            if byte == OSC_INTRO {
                self.in_payload = true;
                self.payload.clear();
                self.overflow = false;
            } else if byte == ESCAPE {
                self.awaiting_intro = true;
            }
            return;
        }
        if byte == ESCAPE {
            self.awaiting_intro = true;
        }
    }

    fn collect(&mut self, byte: u8, events: &mut Vec<(usize, OscEvent)>) {
        if self.overflow {
            return;
        }
        self.payload.push(byte);
        if self.payload.len() > MAX_PAYLOAD_BYTES {
            self.payload.clear();
            self.overflow = true;
            let _ = events;
        }
    }

    fn close(&mut self, offset: usize, events: &mut Vec<(usize, OscEvent)>) {
        let overflowed = self.overflow;
        self.in_payload = false;
        self.awaiting_st = false;
        self.overflow = false;
        if overflowed {
            self.payload.clear();
            return;
        }
        emit(&self.payload, offset, events);
        self.payload.clear();
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
