//! AI 会话统计、Token 用量与上下文占用模型及格式化辅助函数。

use transom_ai::{ChatRole, SessionEntry, SessionEntryKind};
use transom_core::AiTurnMetricsDto;

/// 当前会话及今日/本月的实时推理指标。
#[derive(Clone, Debug, Default)]
pub(crate) struct AiSessionMetrics {
    pub(crate) turns: u32,
    pub(crate) steps: u32,
    pub(crate) model_duration_ms: u64,
    pub(crate) tool_duration_ms: u64,
    pub(crate) ttft_ms_sum: u64,
    pub(crate) ttft_samples: u32,
    pub(crate) last_tps: Option<f32>,
    pub(crate) input_tokens: u64,
    pub(crate) cached_input_tokens: u64,
    pub(crate) output_tokens: u64,
    pub(crate) last_context_tokens: u64,
    pub(crate) today_tokens: u64,
    pub(crate) month_tokens: u64,
    pub(crate) last_breakdown: Option<transom_core::AiContextBreakdownDto>,
}

impl AiSessionMetrics {
    pub(crate) fn set_stored_totals(&mut self, totals: (u64, u64)) {
        self.today_tokens = totals.0;
        self.month_tokens = totals.1;
    }

    pub(crate) fn reset_session(&mut self) {
        let today = self.today_tokens;
        let month = self.month_tokens;
        *self = Self::default();
        self.today_tokens = today;
        self.month_tokens = month;
    }

    /// 从持久化的 JSONL 历史条目恢复会话级推理指标（兼容无 `usage` 的旧会话）。
    pub(crate) fn restore_from_entries(&mut self, entries: &[SessionEntry]) {
        self.reset_session();
        let mut fallback_chars = 0usize;
        for entry in entries {
            match &entry.kind {
                SessionEntryKind::Message {
                    role,
                    text,
                    tokens_per_sec,
                    usage,
                    ..
                } => {
                    fallback_chars = fallback_chars.saturating_add(text.chars().count());
                    match role {
                        ChatRole::User => {
                            self.turns = self.turns.saturating_add(1);
                        }
                        ChatRole::Assistant => {
                            self.steps = self.steps.saturating_add(1);
                            if tokens_per_sec.is_some() {
                                self.last_tps = *tokens_per_sec;
                            }
                            if let Some(u) = usage {
                                self.input_tokens = self.input_tokens.saturating_add(u.input_tokens);
                                self.cached_input_tokens = self
                                    .cached_input_tokens
                                    .saturating_add(u.cached_input_tokens);
                                self.output_tokens =
                                    self.output_tokens.saturating_add(u.output_tokens);
                                self.model_duration_ms =
                                    self.model_duration_ms.saturating_add(u.model_duration_ms);
                                if let Some(ttft) = u.ttft_ms {
                                    self.ttft_ms_sum = self.ttft_ms_sum.saturating_add(ttft);
                                    self.ttft_samples = self.ttft_samples.saturating_add(1);
                                }
                                let turn_ctx = u.input_tokens.saturating_add(u.output_tokens);
                                if turn_ctx > 0 {
                                    self.last_context_tokens = turn_ctx;
                                }
                            }
                        }
                    }
                }
                SessionEntryKind::Reasoning { text } => {
                    fallback_chars = fallback_chars.saturating_add(text.chars().count());
                }
                SessionEntryKind::ToolRecord {
                    summary,
                    output,
                    duration_ms,
                    ..
                } => {
                    self.steps = self.steps.saturating_add(1);
                    self.tool_duration_ms = self.tool_duration_ms.saturating_add(*duration_ms);
                    fallback_chars = fallback_chars
                        .saturating_add(summary.chars().count())
                        .saturating_add(output.chars().count());
                }
                SessionEntryKind::Compaction { summary, .. } => {
                    fallback_chars = fallback_chars.saturating_add(summary.chars().count());
                }
                SessionEntryKind::Header { .. } | SessionEntryKind::Attachment { .. } => {}
            }
        }
        // 兼容老版本未记录 usage 的历史会话：按字符数粗略估算上下文占用，避免重开后显示 0%。
        if self.last_context_tokens == 0 && fallback_chars > 0 {
            self.last_context_tokens = ((fallback_chars / 3).max(1)) as u64;
        }
    }

    pub(crate) fn record_turn(&mut self, tps: Option<f32>, dto: &AiTurnMetricsDto) {
        self.steps = self.steps.saturating_add(1);
        self.model_duration_ms = self.model_duration_ms.saturating_add(dto.model_duration_ms);
        if let Some(ttft) = dto.ttft_ms {
            self.ttft_ms_sum = self.ttft_ms_sum.saturating_add(ttft);
            self.ttft_samples = self.ttft_samples.saturating_add(1);
        }
        if tps.is_some() {
            self.last_tps = tps;
        }
        self.input_tokens = self.input_tokens.saturating_add(dto.input_tokens);
        self.cached_input_tokens = self
            .cached_input_tokens
            .saturating_add(dto.cached_input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(dto.output_tokens);
        let turn_ctx = dto.input_tokens.saturating_add(dto.output_tokens);
        if turn_ctx > 0 {
            self.last_context_tokens = turn_ctx;
        }
        self.today_tokens = dto.today_tokens;
        self.month_tokens = dto.month_tokens;
        if dto.context_breakdown.is_some() {
            self.last_breakdown = dto.context_breakdown.clone();
        }
    }

    pub(crate) fn total_tokens(&self) -> u64 {
        self.input_tokens.saturating_add(self.output_tokens)
    }

    pub(crate) fn uncached_input_tokens(&self) -> u64 {
        self.input_tokens.saturating_sub(self.cached_input_tokens)
    }

    pub(crate) fn cache_hit_percent(&self) -> u64 {
        if self.input_tokens == 0 {
            return 0;
        }
        self.cached_input_tokens
            .saturating_mul(100)
            .checked_div(self.input_tokens)
            .unwrap_or(0)
            .min(100)
    }

    pub(crate) fn avg_ttft_ms(&self) -> Option<u64> {
        self.ttft_ms_sum.checked_div(u64::from(self.ttft_samples))
    }

    pub(crate) fn context_percent(&self, context_window: u32) -> u64 {
        let win = u64::from(context_window.max(1));
        self.last_context_tokens
            .saturating_mul(100)
            .checked_div(win)
            .unwrap_or(0)
            .min(100)
    }
}

pub(crate) fn format_compact_tokens(n: u64) -> String {
    if n >= 1_000_000_000 {
        format!("{:.1}B", n as f64 / 1_000_000_000.0)
    } else if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

pub(crate) fn format_with_commas(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (idx, ch) in s.chars().rev().enumerate() {
        if idx > 0 && idx % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out.chars().rev().collect()
}

pub(crate) fn format_duration_zh(ms: u64) -> String {
    if ms < 60_000 {
        let s = format!("{:.1}", ms as f64 / 1000.0);
        return rust_i18n::t!("ai.dur_secs", s = s).to_string();
    }
    let total_secs = ms / 1000;
    let hours = total_secs / 3600;
    let mins = (total_secs % 3600) / 60;
    let secs = total_secs % 60;
    if hours > 0 {
        rust_i18n::t!("ai.dur_hours_mins_secs", h = hours, m = mins, s = secs).to_string()
    } else {
        rust_i18n::t!("ai.dur_mins_secs", m = mins, s = secs).to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::{AiSessionMetrics, format_compact_tokens, format_duration_zh, format_with_commas};
    use transom_core::AiTurnMetricsDto;

    #[test]
    fn formats_tokens_commas_and_durations_cleanly() {
        assert_eq!(format_compact_tokens(850), "850");
        assert_eq!(format_compact_tokens(87_200_000), "87.2M");
        assert_eq!(format_compact_tokens(1_200_000_000), "1.2B");
        assert_eq!(format_with_commas(87_570_086), "87,570,086");
        assert!(!format_duration_zh(6_200).is_empty());
        assert!(!format_duration_zh(3_874_000).is_empty());
    }

    #[test]
    fn accumulates_turn_metrics_and_preserves_daily_totals_on_reset() {
        let mut m = AiSessionMetrics::default();
        m.record_turn(
            Some(166.0),
            &AiTurnMetricsDto {
                input_tokens: 1_000,
                cached_input_tokens: 950,
                output_tokens: 200,
                model_duration_ms: 1_200,
                ttft_ms: Some(300),
                today_tokens: 5_000,
                month_tokens: 20_000,
                context_breakdown: None,
            },
        );
        assert_eq!(m.cache_hit_percent(), 95);
        assert_eq!(m.total_tokens(), 1_200);
        assert_eq!(m.context_percent(12_000), 10);

        m.reset_session();
        assert_eq!(m.total_tokens(), 0);
        assert_eq!(m.today_tokens, 5_000);
        assert_eq!(m.month_tokens, 20_000);
    }

    #[test]
    fn restores_metrics_from_persisted_entries_and_legacy_fallback() {
        use transom_ai::{ChatRole, SessionEntry, SessionEntryKind, TurnUsageRecord};

        let mut m = AiSessionMetrics::default();
        m.set_stored_totals((9_000, 30_000));

        let entries = vec![
            SessionEntry::new(
                None,
                SessionEntryKind::Message {
                    role: ChatRole::User,
                    text: "巡检一下".to_owned(),
                    images: vec![],
                    model: None,
                    tokens_per_sec: None,
                    usage: None,
                },
            ),
            SessionEntry::new(
                None,
                SessionEntryKind::Message {
                    role: ChatRole::Assistant,
                    text: "已完成只读巡检。".to_owned(),
                    images: vec![],
                    model: Some("deepseek-v4.1-flash".to_owned()),
                    tokens_per_sec: Some(193.3),
                    usage: Some(TurnUsageRecord {
                        input_tokens: 10_000,
                        cached_input_tokens: 7_900,
                        output_tokens: 1_000,
                        model_duration_ms: 1_600,
                        ttft_ms: Some(240),
                    }),
                },
            ),
        ];

        m.restore_from_entries(&entries);
        assert_eq!(m.turns, 1);
        assert_eq!(m.steps, 1);
        assert_eq!(m.last_tps, Some(193.3));
        assert_eq!(m.total_tokens(), 11_000);
        assert_eq!(m.cache_hit_percent(), 79);
        assert_eq!(m.last_context_tokens, 11_000);
        assert_eq!(m.avg_ttft_ms(), Some(240));
        assert_eq!(m.today_tokens, 9_000);
        assert_eq!(m.month_tokens, 30_000);

        let legacy_entries = vec![SessionEntry::new(
            None,
            SessionEntryKind::Message {
                role: ChatRole::Assistant,
                text: "这是一段没有 usage 字段的老历史消息".to_owned(),
                images: vec![],
                model: None,
                tokens_per_sec: Some(120.0),
                usage: None,
            },
        )];
        m.restore_from_entries(&legacy_entries);
        assert_eq!(m.last_tps, Some(120.0));
        assert!(m.last_context_tokens > 0);
    }
}
