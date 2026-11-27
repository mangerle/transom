//! AI 三层记忆、自定义技能目录、审计日志与 Token 用量统计持久化。

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use transom_ai::{
    AiAuditEntry, AiError, MemorySnapshot, read_memory_file_limited, resolve_sandbox_path,
};

use super::{AiStore, sanitize_id_filename};
use crate::file_io::write_atomic;

impl AiStore {
    /// 读取三层记忆中的全局记忆与指定主机记忆。
    pub fn load_memory(&self, host_id: Option<&str>) -> Result<MemorySnapshot, AiError> {
        let mem_root = self.root.join("memory");
        let global_path = resolve_sandbox_path(&mem_root, Path::new("global.md"))?;
        let global_markdown = read_memory_file_limited(&global_path)?;

        let host_markdown = if let Some(id) = host_id {
            let safe_name = sanitize_id_filename(id, "md");
            let host_dir = mem_root.join("hosts");
            let host_path = resolve_sandbox_path(&host_dir, Path::new(&safe_name))?;
            read_memory_file_limited(&host_path)?
        } else {
            String::new()
        };
        Ok(MemorySnapshot {
            global_markdown,
            host_markdown,
        })
    }

    /// 保存全局记忆 Markdown。
    pub fn save_global_memory(&self, markdown: &str) -> Result<(), AiError> {
        let mem_root = self.root.join("memory");
        let path = resolve_sandbox_path(&mem_root, Path::new("global.md"))?;
        write_atomic(&path, markdown.as_bytes()).map_err(|err| AiError::StorageFailed {
            target: path.display().to_string(),
            reason: err.to_string(),
        })
    }

    /// 保存指定主机的长期记忆 Markdown。
    pub fn save_host_memory(&self, host_id: &str, markdown: &str) -> Result<(), AiError> {
        let host_dir = self.root.join("memory").join("hosts");
        let safe_name = sanitize_id_filename(host_id, "md");
        let path = resolve_sandbox_path(&host_dir, Path::new(&safe_name))?;
        write_atomic(&path, markdown.as_bytes()).map_err(|err| AiError::StorageFailed {
            target: path.display().to_string(),
            reason: err.to_string(),
        })
    }

    /// 审计日志文件路径。
    pub fn append_audit(&self, entry: &AiAuditEntry) -> Result<(), AiError> {
        let path = self.root.join("audit.jsonl");
        entry.append_to_file(&path)
    }

    /// 用户自定义技能目录。
    pub fn custom_skills_dir(&self) -> PathBuf {
        self.root.join("skills")
    }

    /// 读取今日与本月累计 Token 用量（跨日/跨月自动归零）。
    pub fn load_usage_totals(&self) -> (u64, u64) {
        let _guard = self.usage_lock.lock();
        let (day, month) = current_day_and_month_keys();
        let mut totals = self.read_usage_totals();
        totals.rollover(&day, &month);
        (totals.today_tokens, totals.month_tokens)
    }

    /// 累加本次推理消耗的 Token 并持久化到 `usage_totals.json`。
    pub fn record_token_usage(&self, added_tokens: u64) -> (u64, u64) {
        let _guard = self.usage_lock.lock();
        let (day, month) = current_day_and_month_keys();
        let mut totals = self.read_usage_totals();
        totals.rollover(&day, &month);
        if added_tokens > 0 {
            totals.today_tokens = totals.today_tokens.saturating_add(added_tokens);
            totals.month_tokens = totals.month_tokens.saturating_add(added_tokens);
            let path = self.root.join("usage_totals.json");
            if let Ok(payload) = serde_json::to_string_pretty(&totals) {
                let _ = write_atomic(&path, payload.as_bytes());
            }
        }
        (totals.today_tokens, totals.month_tokens)
    }

    fn read_usage_totals(&self) -> AiUsageTotals {
        let path = self.root.join("usage_totals.json");
        let Ok(text) = fs::read_to_string(&path) else {
            return AiUsageTotals::default();
        };
        serde_json::from_str(&text).unwrap_or_default()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct AiUsageTotals {
    #[serde(default)]
    day_key: String,
    #[serde(default)]
    today_tokens: u64,
    #[serde(default)]
    month_key: String,
    #[serde(default)]
    month_tokens: u64,
}

impl AiUsageTotals {
    fn rollover(&mut self, day: &str, month: &str) {
        if self.day_key != day {
            self.day_key = day.to_owned();
            self.today_tokens = 0;
        }
        if self.month_key != month {
            self.month_key = month.to_owned();
            self.month_tokens = 0;
        }
    }
}

fn current_day_and_month_keys() -> (String, String) {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = (secs / 86_400) as i64;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = y + i64::from(m <= 2);
    (
        format!("{year:04}-{m:02}-{d:02}"),
        format!("{year:04}-{m:02}"),
    )
}

#[cfg(test)]
mod tests {
    use super::super::AiStore;
    use super::AiUsageTotals;

    #[test]
    fn memory_save_and_load_isolates_hosts() {
        let dir = tempfile::tempdir().expect("创建临时目录失败");
        let store = AiStore::with_root(dir.path().to_path_buf());
        store
            .save_global_memory("优先使用 ss")
            .expect("保存全局记忆失败");
        store
            .save_host_memory("host-1", "Nginx 在 /etc/nginx")
            .expect("保存主机记忆失败");

        let snap = store.load_memory(Some("host-1")).expect("读取记忆失败");
        assert!(snap.global_markdown.contains("优先使用 ss"));
        assert!(snap.host_markdown.contains("Nginx"));

        let other = store
            .load_memory(Some("host-2"))
            .expect("读取空主机记忆失败");
        assert!(other.host_markdown.is_empty());
    }

    #[test]
    fn usage_totals_accumulate_and_rollover_on_new_day_or_month() {
        let dir = tempfile::tempdir().expect("创建临时目录失败");
        let store = AiStore::with_root(dir.path().to_path_buf());
        assert_eq!(store.load_usage_totals(), (0, 0));
        assert_eq!(store.record_token_usage(1_500), (1_500, 1_500));
        assert_eq!(store.record_token_usage(500), (2_000, 2_000));
        assert_eq!(store.load_usage_totals(), (2_000, 2_000));

        let mut totals = AiUsageTotals {
            day_key: "1999-01-01".to_owned(),
            today_tokens: 999,
            month_key: "1999-01".to_owned(),
            month_tokens: 8_888,
        };
        totals.rollover("2026-11-26", "2026-11");
        assert_eq!(totals.today_tokens, 0);
        assert_eq!(totals.month_tokens, 0);
    }
}
