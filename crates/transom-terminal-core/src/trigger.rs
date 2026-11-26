//! 终端输出流触发器执行引擎与正则匹配器。
//!
//! 设计说明（Why）：
//! - 实现初衷：为终端输出流提供自动模式匹配与动作响应能力，支持根据输出命中预设模式自动执行应答、响铃或通知；
//! - 核心优势：
//!   1. 预编译模式缓存：预先编译正则表达式，避免高频字节流到达时反复解析与构建正则状态机；
//!   2. 增量行与块匹配缓冲：维护有限大小（最大 4096 字节）的行结尾文本缓冲，兼顾流式分片边界（跨 packet 匹配）与内存安全；
//!   3. 剥除 ANSI 转义序列：输出流中往往包含光标控制与着色控制码，正则扫描前剥除转义字符，避免污染用户编写的业务模式；
//!   4. 触发动作去噪与节流：支持对同一规则设定最小触发间隔，杜绝密集刷屏引发通知轰炸或死循环应答。

use regex::Regex;
use std::time::{Duration, Instant};
use transom_core::{TriggerAction, TriggerId, TriggerSettings};

/// 单条已编译的触发器规则。
#[derive(Debug, Clone)]
pub struct CompiledTrigger {
    pub id: TriggerId,
    pub name: String,
    pub regex: Regex,
    pub action: TriggerAction,
    /// 上一次成功命中的时间戳，用于去抖与防死锁节流。
    pub last_fired: Option<Instant>,
    /// 已消费到的绝对字节位置（自缓冲纪元起算）。同一段文本被后续 chunk
    /// 反复扫描时，据此跳过旧命中，避免无换行提示被重复应答。
    last_scanned_end: usize,
}

/// 终端触发器执行引擎。
#[derive(Debug, Default)]
pub struct TriggerEngine {
    /// 触发器总开关。
    enabled: bool,
    /// 预编译的有效规则列表。
    compiled_rules: Vec<CompiledTrigger>,
    /// 跨 packet 行缓冲（最多保留 4096 字节，防止无换行时无界增长）。
    line_buffer: String,
    /// 缓冲区首字节对应的绝对偏移；截断或丢弃已换行前缀时同步累加。
    trimmed: usize,
}

/// 匹配结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerMatchResult {
    pub id: TriggerId,
    pub name: String,
    pub action: TriggerAction,
}

impl TriggerEngine {
    /// 缓冲行最大保留字节数。
    const MAX_BUFFER_BYTES: usize = 4096;
    /// 扫描时向前回看的重叠窗口：让跨 chunk 的提示仍能拼接匹配，同时避免
    /// 整段重扫导致旧命中与新文本混在一起判断。
    const SCAN_OVERLAP: usize = 256;
    /// 单个规则最小重新触发间隔（500ms），防高频刷屏轰炸。
    const MIN_TRIGGER_INTERVAL: Duration = Duration::from_millis(500);

    /// 从设置重载并编译全部触发器规则。
    pub fn reload(&mut self, settings: &TriggerSettings) {
        self.enabled = settings.enabled;
        self.compiled_rules.clear();
        self.line_buffer.clear();
        self.trimmed = 0;

        if !settings.enabled {
            return;
        }

        for rule in &settings.rules {
            if !rule.enabled || rule.pattern.trim().is_empty() {
                continue;
            }

            let pattern = if rule.case_sensitive {
                rule.pattern.clone()
            } else {
                format!("(?i){}", rule.pattern)
            };

            match Regex::new(&pattern) {
                Ok(regex) => {
                    self.compiled_rules.push(CompiledTrigger {
                        id: rule.id,
                        name: if rule.name.trim().is_empty() {
                            rule.pattern.clone()
                        } else {
                            rule.name.clone()
                        },
                        regex,
                        action: rule.action.clone(),
                        last_fired: None,
                        last_scanned_end: 0,
                    });
                }
                Err(error) => {
                    log::warn!(
                        "编译终端触发器规则 '{}' 失败，模式: '{}'，原因: {error}",
                        rule.name,
                        rule.pattern
                    );
                }
            }
        }
    }

    /// 摄入终端输出字节片段并执行触发器检测。
    ///
    /// 返回所有在本轮输出中成功命中的动作列表。
    pub fn process_bytes(&mut self, bytes: &[u8]) -> Vec<TriggerMatchResult> {
        if !self.enabled || self.compiled_rules.is_empty() || bytes.is_empty() {
            return Vec::new();
        }

        // 转为有损 UTF-8 文本并剥除 ANSI 转义序列
        let raw_text = String::from_utf8_lossy(bytes);
        let stripped = strip_ansi_escapes(&raw_text);
        if stripped.is_empty() {
            return Vec::new();
        }

        self.line_buffer.push_str(&stripped);

        // 控制缓冲长度；截断点必须落在字符边界，否则多字节字符会被劈开。
        if self.line_buffer.len() > Self::MAX_BUFFER_BYTES {
            let mut keep_start = self.line_buffer.len() - Self::MAX_BUFFER_BYTES / 2;
            while !self.line_buffer.is_char_boundary(keep_start) {
                keep_start += 1;
            }
            self.line_buffer = self.line_buffer.split_off(keep_start);
            self.trimmed = self.trimmed.saturating_add(keep_start);
        }

        let mut matches = Vec::new();
        let now = Instant::now();
        let buffer = &self.line_buffer;

        for rule in &mut self.compiled_rules {
            // 节流检查
            if let Some(last) = rule.last_fired
                && now.duration_since(last) < Self::MIN_TRIGGER_INTERVAL
            {
                continue;
            }

            // 旧文本已被丢弃：消费水位随纪元前移，新缓冲里的匹配都算新命中。
            if rule.last_scanned_end < self.trimmed {
                rule.last_scanned_end = self.trimmed;
            }

            let Some((captures, absolute_end)) =
                Self::find_fresh_match(rule, buffer, self.trimmed, Self::SCAN_OVERLAP)
            else {
                continue;
            };
            rule.last_scanned_end = absolute_end;
            rule.last_fired = Some(now);

            let action = match &rule.action {
                TriggerAction::AutoResponse(template) => {
                    let expanded = expand_captures(template, &captures);
                    TriggerAction::AutoResponse(expanded)
                }
                TriggerAction::Notification(template) => {
                    let expanded = if template.trim().is_empty() {
                        rust_i18n::t!(
                            "terminal.trigger.default_notification",
                            name = rule.name.as_str()
                        )
                        .to_string()
                    } else {
                        expand_captures(template, &captures)
                    };
                    TriggerAction::Notification(expanded)
                }
                TriggerAction::Bell => TriggerAction::Bell,
                TriggerAction::Unknown => TriggerAction::Unknown,
            };

            matches.push(TriggerMatchResult {
                id: rule.id,
                name: rule.name.clone(),
                action,
            });
        }

        // 若缓冲区末尾包含换行，保留未换行残余；丢弃的前缀同步计入绝对偏移。
        if let Some(last_newline) = self.line_buffer.rfind('\n') {
            let drop_len = last_newline + 1;
            self.trimmed = self.trimmed.saturating_add(drop_len);
            let _ = self.line_buffer.drain(..drop_len);
        }

        matches
    }

    /// 从消费水位之后（含回看窗口）寻找第一个尚未消费的匹配。
    ///
    /// 整段重扫会把"上次已应答的无换行提示"再次交给规则，造成每个 chunk 都
    /// 重复外发；这里逐个跳过结束位置不前进的旧命中，直到找到新文本形成的
    /// 匹配为止。
    fn find_fresh_match<'a>(
        rule: &CompiledTrigger,
        buffer: &'a str,
        trimmed: usize,
        overlap: usize,
    ) -> Option<(regex::Captures<'a>, usize)> {
        let mut search_from = rule
            .last_scanned_end
            .saturating_sub(overlap)
            .max(trimmed)
            .saturating_sub(trimmed);
        while search_from < buffer.len() && !buffer.is_char_boundary(search_from) {
            search_from += 1;
        }

        while search_from <= buffer.len() {
            let captures = rule.regex.captures(&buffer[search_from..])?;
            let full = captures.get(0)?;
            let absolute_end = trimmed + search_from + full.end();
            if absolute_end > rule.last_scanned_end {
                return Some((captures, absolute_end));
            }
            let mut next = search_from + full.end();
            if next <= search_from {
                break;
            }
            while next < buffer.len() && !buffer.is_char_boundary(next) {
                next += 1;
            }
            search_from = next;
        }
        None
    }
}

/// 替换模板中的捕获组（如 `$0` 全文，`$1` 第一组）。
fn expand_captures(template: &str, captures: &regex::Captures) -> String {
    let mut result = template.to_string();
    for (i, m) in captures.iter().enumerate() {
        if let Some(matched) = m {
            let placeholder = format!("${i}");
            result = result.replace(&placeholder, matched.as_str());
        }
    }
    result
}

/// 快速移除 ANSI CSI/OSC 控制序列，返回纯净文本。
fn strip_ansi_escapes(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\x1b'
            && let Some(&next) = chars.peek()
        {
            if next == '[' {
                chars.next();
                // CSI 序列以 0x40-0x7E 结尾
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
                continue;
            } else if next == ']' {
                chars.next();
                // OSC 序列以 BEL(\x07) 或 ESC \ 结尾
                while let Some(c) = chars.next() {
                    if c == '\x07' {
                        break;
                    }
                    if c == '\x1b' && chars.peek() == Some(&'\\') {
                        chars.next();
                        break;
                    }
                }
                continue;
            }
        }
        out.push(ch);
    }

    out
}

#[cfg(test)]
mod tests {
    use super::{TriggerAction, TriggerEngine, TriggerSettings, strip_ansi_escapes};
    use std::time::{Duration, Instant};
    use transom_core::TriggerRule;

    fn engine_with(pattern: &str, action: TriggerAction) -> TriggerEngine {
        let mut settings = TriggerSettings::default();
        settings
            .rules
            .push(TriggerRule::new("测试规则", pattern, action));
        let mut engine = TriggerEngine::default();
        engine.reload(&settings);
        engine
    }

    fn clear_throttle(engine: &mut TriggerEngine) {
        let past = Instant::now()
            .checked_sub(TriggerEngine::MIN_TRIGGER_INTERVAL + Duration::from_millis(1))
            .expect("测试回拨时间不应溢出");
        engine.compiled_rules[0].last_fired = Some(past);
    }

    /// 超长无换行多字节输出截断时不得劈开字符（曾按字节下标切片 panic）。
    #[test]
    fn long_multibyte_line_is_truncated_on_char_boundary() {
        let mut engine = engine_with(r"error", TriggerAction::Bell);
        let chunk = "中".repeat(3000);

        let matches = engine.process_bytes(chunk.as_bytes());

        assert!(matches.is_empty());
        assert!(engine.line_buffer.len() <= TriggerEngine::MAX_BUFFER_BYTES);
    }

    /// 无换行提示只应答一次：同一段文本被后续 chunk 反复扫描时不得重复外发。
    #[test]
    fn unterminated_prompt_fires_only_once() {
        let mut engine = engine_with(r"password:", TriggerAction::AutoResponse("secret".into()));

        assert_eq!(engine.process_bytes(b"password:").len(), 1);
        // 绕过节流，单独验证消费水位；否则 500ms 节流会掩盖水位失效。
        clear_throttle(&mut engine);

        assert!(engine.process_bytes(b" still waiting").is_empty());
    }

    /// 同一缓冲里出现新的提示时仍要再次应答，水位不能把新文本一起吞掉。
    #[test]
    fn new_prompt_in_same_buffer_still_fires() {
        let mut engine = engine_with(r"password:", TriggerAction::AutoResponse("secret".into()));

        assert_eq!(engine.process_bytes(b"password:").len(), 1);
        clear_throttle(&mut engine);

        assert_eq!(engine.process_bytes(b" again password:").len(), 1);
    }

    /// 换行清理旧文本后，新行里的同一提示必须再次触发。
    #[test]
    fn prompt_after_newline_fires_again() {
        let mut engine = engine_with(r"password:", TriggerAction::AutoResponse("secret".into()));

        assert_eq!(engine.process_bytes(b"password:").len(), 1);
        clear_throttle(&mut engine);
        assert!(engine.process_bytes(b"\n").is_empty());
        clear_throttle(&mut engine);

        assert_eq!(engine.process_bytes(b"password:").len(), 1);
    }

    #[test]
    fn test_strip_ansi_escapes() {
        let raw = "\x1b[31;1mError:\x1b[0m Connection refused\x1b[?25h";
        assert_eq!(strip_ansi_escapes(raw), "Error: Connection refused");
    }

    #[test]
    fn test_trigger_engine_auto_response_and_expansion() {
        let mut settings = TriggerSettings::default();
        let rule = TriggerRule::new(
            "密码输入提示",
            r"password for (\w+):",
            TriggerAction::AutoResponse("secret-for-$1".into()),
        );
        settings.rules.push(rule);

        let mut engine = TriggerEngine::default();
        engine.reload(&settings);

        let output = b"Please enter password for root: ";
        let matches = engine.process_bytes(output);
        assert_eq!(matches.len(), 1);
        assert_eq!(
            matches[0].action,
            TriggerAction::AutoResponse("secret-for-root".into())
        );
    }
}
