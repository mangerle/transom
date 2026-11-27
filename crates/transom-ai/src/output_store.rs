//! 会话级超大工具输出句柄池（`ToolOutputStore`）。
//!
//! 设计原理（Why）：
//! - 远端日志、容器输出或大配置文件动辄数万字符，直接截断会丢失中间关键报错；
//! - 超出阈值的输出存入有界内存池并返回 `output_id` 与头尾预览，模型可按行偏移或关键字按需分页读取；
//! - 采用条目数上限（64 条）与总字节上限（16 MiB）双保险 LRU 淘汰，彻底杜绝内存无限增长。

use std::collections::{HashMap, VecDeque};

use parking_lot::Mutex;
use uuid::Uuid;

use crate::compaction::{DEFAULT_TOOL_OUTPUT_LIMIT, strip_ansi, truncate_tool_output};
use crate::error::ToolError;
use crate::redact::redact_secrets_for_model;

/// 句柄池最大缓存条目数。
pub const MAX_STORED_OUTPUTS: usize = 64;

/// 句柄池总字节上限（16 MiB）。
pub const MAX_STORE_TOTAL_BYTES: usize = 16 * 1024 * 1024;

/// 单次分页读取默认行数。
pub const DEFAULT_READ_LINES: usize = 120;

/// 单次分页读取最大行数。
pub const MAX_READ_LINES: usize = 300;

#[derive(Clone, Debug)]
struct StoredOutput {
    tool_name: String,
    text: String,
    line_count: usize,
    byte_len: usize,
}

/// 线程安全的会话级工具输出句柄池。
#[derive(Debug, Default)]
pub struct ToolOutputStore {
    inner: Mutex<StoreInner>,
}

#[derive(Debug, Default)]
struct StoreInner {
    entries: HashMap<String, StoredOutput>,
    order: VecDeque<String>,
    total_bytes: usize,
}

impl ToolOutputStore {
    /// 创建空的输出句柄池。
    pub fn new() -> Self {
        Self::default()
    }

    /// 清洗 ANSI 并遮蔽明文密钥，超限时落池返回带 `output_id` 的头尾预览。
    pub fn store_and_preview(
        &self,
        tool_name: &str,
        raw: &str,
        max_preview_chars: usize,
    ) -> String {
        let cleaned = redact_secrets_for_model(&strip_ansi(raw));
        let char_count = cleaned.chars().count();
        if char_count <= max_preview_chars || max_preview_chars < 64 {
            return cleaned;
        }

        let text = cleaned;
        let line_count = text.lines().count().max(1);
        let byte_len = text.len();
        let id = format!("out-{}", &Uuid::new_v4().simple().to_string()[..8]);

        self.insert(
            id.clone(),
            StoredOutput {
                tool_name: tool_name.to_owned(),
                text: text.clone(),
                line_count,
                byte_len,
            },
        );

        let preview = truncate_tool_output(&text, max_preview_chars);
        format!(
            "{preview}\n\n[完整输出已缓存: output_id={id}，共 {line_count} 行 / {byte_len} 字节；\
            如需查看中间片段或检索关键字，请调用 read_tool_output(output_id=\"{id}\")]"
        )
    }

    /// 按行偏移或关键字分页读取已缓存的工具输出。
    ///
    /// # Errors
    /// 当 `output_id` 不存在或已被淘汰时返回 [`ToolError::InvalidArguments`]。
    pub fn read(
        &self,
        output_id: &str,
        offset_line: usize,
        limit_lines: usize,
        pattern: Option<&str>,
    ) -> Result<String, ToolError> {
        let guard = self.inner.lock();
        let Some(entry) = guard.entries.get(output_id) else {
            return Err(ToolError::InvalidArguments {
                tool: "read_tool_output",
                reason: format!("未找到输出句柄 {output_id}（可能已过期或被容量淘汰）"),
            });
        };

        let limit = if limit_lines == 0 {
            DEFAULT_READ_LINES
        } else {
            limit_lines.clamp(1, MAX_READ_LINES)
        };
        let total = entry.line_count;
        let query = pattern.map(str::trim).filter(|s| !s.is_empty());

        let body = if let Some(pat) = query {
            let pat_lower = pat.to_ascii_lowercase();
            let matched: Vec<String> = entry
                .text
                .lines()
                .enumerate()
                .filter(|(_, line)| line.to_ascii_lowercase().contains(&pat_lower))
                .skip(offset_line)
                .take(limit)
                .map(|(idx, line)| format!("L{}: {line}", idx + 1))
                .collect();
            if matched.is_empty() {
                format!(
                    "[句柄 {output_id}（工具: {}，共 {total} 行）未匹配到关键字 \"{pat}\"（offset={offset_line}）]",
                    entry.tool_name
                )
            } else {
                format!(
                    "[句柄 {output_id} 关键字 \"{pat}\" 匹配结果，共返回 {} 行（offset={offset_line}）]\n{}",
                    matched.len(),
                    matched.join("\n")
                )
            }
        } else {
            let start = offset_line.min(total);
            let slice: Vec<String> = entry
                .text
                .lines()
                .enumerate()
                .skip(start)
                .take(limit)
                .map(|(idx, line)| format!("L{}: {line}", idx + 1))
                .collect();
            let shown = slice.len();
            let remaining = total.saturating_sub(start.saturating_add(shown));
            format!(
                "[句柄 {output_id}（工具: {}），显示 L{}..L{} / 共 {total} 行，剩余 {remaining} 行]\n{}",
                entry.tool_name,
                start.saturating_add(1),
                start.saturating_add(shown),
                slice.join("\n")
            )
        };

        Ok(truncate_tool_output(&body, DEFAULT_TOOL_OUTPUT_LIMIT))
    }

    fn insert(&self, id: String, item: StoredOutput) {
        let mut guard = self.inner.lock();
        // 单条输出本身超过总上限时只保留末尾 16 MiB 以内，避免撑爆池子。
        if item.byte_len > MAX_STORE_TOTAL_BYTES {
            return;
        }
        while guard.order.len() >= MAX_STORED_OUTPUTS
            || guard.total_bytes.saturating_add(item.byte_len) > MAX_STORE_TOTAL_BYTES
        {
            let Some(oldest_id) = guard.order.pop_front() else {
                break;
            };
            if let Some(removed) = guard.entries.remove(&oldest_id) {
                guard.total_bytes = guard.total_bytes.saturating_sub(removed.byte_len);
            }
        }
        guard.total_bytes = guard.total_bytes.saturating_add(item.byte_len);
        guard.order.push_back(id.clone());
        guard.entries.insert(id, item);
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_STORED_OUTPUTS, ToolOutputStore};

    #[test]
    fn small_output_passes_through_without_handle() {
        let store = ToolOutputStore::new();
        let out = store.store_and_preview("remote_exec", "\x1b[32mok\x1b[0m", 1_000);
        assert_eq!(out, "ok");
    }

    #[test]
    fn large_output_spills_to_store_and_supports_paging_and_search() {
        let store = ToolOutputStore::new();
        let mut lines = Vec::with_capacity(300);
        for i in 1..=300 {
            if i == 150 {
                lines.push("line 150 FATAL_OOM_MARKER happened here".to_owned());
            } else {
                lines.push(format!("line {i:03} padding {}", "x".repeat(40)));
            }
        }
        let raw = lines.join("\n");
        let preview = store.store_and_preview("remote_exec", &raw, 800);
        let marker = "output_id=";
        let start = preview.find(marker).expect("必须包含 output_id") + marker.len();
        let id: String = preview[start..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect();

        let page = store.read(&id, 148, 5, None).expect("分页读取应成功");
        assert!(page.contains("L150: line 150 FATAL_OOM_MARKER"));

        let search = store
            .read(&id, 0, 10, Some("fatal_oom_marker"))
            .expect("关键字检索应成功");
        assert!(search.contains("L150: line 150 FATAL_OOM_MARKER"));
    }

    #[test]
    fn evicts_oldest_when_capacity_exceeded() {
        let store = ToolOutputStore::new();
        let payload = "a".repeat(200);
        let mut first_id = String::new();
        for idx in 0..=MAX_STORED_OUTPUTS {
            let preview = store.store_and_preview("remote_exec", &payload, 100);
            if idx == 0 {
                let pos = preview.find("output_id=").expect("包含句柄") + 10;
                first_id = preview[pos..]
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
                    .collect();
            }
        }
        assert!(store.read(&first_id, 0, 10, None).is_err());
    }
}
