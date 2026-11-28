//! AI 三层记忆、自定义技能目录、审计日志与 Token 用量统计持久化。

use chrono::{DateTime, Local, TimeZone};
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use transom_ai_contracts::{
    AiError, MemorySnapshot, read_memory_file_limited, resolve_sandbox_path,
};

use super::{AiStore, resolve_id_file_path};
use crate::file_io::write_atomic;

impl AiStore {
    /// 读取三层记忆中的全局记忆与指定主机记忆。
    pub fn load_memory(&self, host_id: Option<&str>) -> Result<MemorySnapshot, AiError> {
        let mem_root = self.root.join("memory");
        let global_path = resolve_sandbox_path(&mem_root, Path::new("global.md"))?;
        let global_markdown = read_memory_file_limited(&global_path)?;

        let host_markdown = if let Some(id) = host_id {
            let host_dir = mem_root.join("hosts");
            let host_path = resolve_id_file_path(&host_dir, id, "md")?;
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
        let path = resolve_id_file_path(&host_dir, host_id, "md")?;
        write_atomic(&path, markdown.as_bytes()).map_err(|err| AiError::StorageFailed {
            target: path.display().to_string(),
            reason: err.to_string(),
        })
    }

    /// 用户自定义技能目录。
    pub fn custom_skills_dir(&self) -> PathBuf {
        self.root.join("skills")
    }

    /// 带目录 mtime 缓存地加载全部技能并应用禁用列表，避免每次提问重复扫描磁盘。
    pub fn load_skills_cached(
        &self,
        disabled_ids: &[String],
    ) -> Vec<transom_ai_contracts::SkillDefinition> {
        let dir = self.custom_skills_dir();
        let dir_mtime = fs::metadata(&dir).and_then(|m| m.modified()).ok();
        let mut skills = {
            let mut guard = self.skills_cache.lock();
            if let Some((cached_mtime, cached_list)) = guard.as_ref()
                && *cached_mtime == dir_mtime
            {
                cached_list.clone()
            } else {
                let loaded = transom_ai_contracts::load_all_skills(Some(&dir));
                *guard = Some((dir_mtime, loaded.clone()));
                loaded
            }
        };

        let disabled: std::collections::HashSet<String> = disabled_ids
            .iter()
            .map(|s| s.trim().trim_start_matches('/').to_ascii_lowercase())
            .filter(|s| !s.is_empty())
            .collect();
        for skill in &mut skills {
            let id_lc = skill.id.to_ascii_lowercase();
            let cmd_lc = skill.command.trim_start_matches('/').to_ascii_lowercase();
            skill.enabled = !disabled.contains(&id_lc) && !disabled.contains(&cmd_lc);
        }
        skills
    }

    /// 读取今日与本月累计 Token 用量（跨日/跨月自动归零）。
    ///
    /// # Errors
    /// 用量文件读取或反序列化失败时返回 [`AiError::StorageFailed`]。
    pub fn load_usage_totals(&self) -> Result<(u64, u64), AiError> {
        let _guard = self.usage_lock.lock();
        let (day, month) = current_day_and_month_keys();
        let mut totals = self.read_usage_totals()?;
        totals.rollover(&day, &month);
        Ok((totals.today_tokens, totals.month_tokens))
    }

    /// 累加本次推理消耗的 Token 并持久化到 `usage_totals.json`。
    ///
    /// # Errors
    /// 读取旧统计、序列化或原子写入失败时返回 [`AiError::StorageFailed`]。
    pub fn record_token_usage(&self, added_tokens: u64) -> Result<(u64, u64), AiError> {
        let _guard = self.usage_lock.lock();
        let (day, month) = current_day_and_month_keys();
        let mut totals = self.read_usage_totals()?;
        totals.rollover(&day, &month);
        if added_tokens > 0 {
            totals.today_tokens = totals.today_tokens.saturating_add(added_tokens);
            totals.month_tokens = totals.month_tokens.saturating_add(added_tokens);
            let path = self.root.join("usage_totals.json");
            let payload =
                serde_json::to_vec_pretty(&totals).map_err(|err| AiError::StorageFailed {
                    target: path.display().to_string(),
                    reason: err.to_string(),
                })?;
            write_atomic(&path, &payload).map_err(|err| AiError::StorageFailed {
                target: path.display().to_string(),
                reason: err.to_string(),
            })?;
        }
        Ok((totals.today_tokens, totals.month_tokens))
    }

    fn read_usage_totals(&self) -> Result<AiUsageTotals, AiError> {
        let path = self.root.join("usage_totals.json");
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Ok(AiUsageTotals::default());
            }
            Err(err) => {
                return Err(AiError::StorageFailed {
                    target: path.display().to_string(),
                    reason: err.to_string(),
                });
            }
        };
        serde_json::from_str(&text).map_err(|err| AiError::StorageFailed {
            target: path.display().to_string(),
            reason: err.to_string(),
        })
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
    day_and_month_keys_at(&Local::now())
}

fn day_and_month_keys_at<Tz>(now: &DateTime<Tz>) -> (String, String)
where
    Tz: TimeZone,
    Tz::Offset: std::fmt::Display,
{
    (
        now.format("%Y-%m-%d").to_string(),
        now.format("%Y-%m").to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::super::AiStore;
    use super::{AiUsageTotals, day_and_month_keys_at};
    use chrono::{FixedOffset, TimeZone};

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

    #[cfg(unix)]
    #[test]
    fn audit_log_is_owner_only() {
        use std::os::unix::fs::PermissionsExt as _;
        use transom_ai_contracts::{AiAuditEntry, ToolRisk};

        let dir = tempfile::tempdir().expect("创建临时目录失败");
        let store = AiStore::with_root(dir.path().to_path_buf());
        store
            .append_audit(&AiAuditEntry {
                timestamp: "2026-11-27T00:00:00Z".to_owned(),
                conversation_id: "conv-1".to_owned(),
                host_label: "host-1".to_owned(),
                tool_name: "remote_exec".to_owned(),
                risk: ToolRisk::SafeRead,
                summary: "uptime".to_owned(),
                approved: true,
                ok: true,
                duration_ms: 1,
            })
            .expect("写入审计失败");
        let mode = std::fs::metadata(dir.path().join("audit.jsonl"))
            .expect("读取审计文件属性失败")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn day_and_month_keys_follow_local_offset() {
        let offset = FixedOffset::east_opt(8 * 60 * 60).expect("固定时区应合法");
        let local_midnight = offset
            .with_ymd_and_hms(2026, 1, 1, 0, 30, 0)
            .single()
            .expect("本地时间应唯一");
        assert_eq!(
            day_and_month_keys_at(&local_midnight),
            ("2026-01-01".to_owned(), "2026-01".to_owned())
        );
    }

    #[test]
    fn usage_totals_accumulate_and_rollover_on_new_day_or_month() {
        let dir = tempfile::tempdir().expect("创建临时目录失败");
        let store = AiStore::with_root(dir.path().to_path_buf());
        assert_eq!(store.load_usage_totals().expect("读取用量失败"), (0, 0));
        assert_eq!(
            store.record_token_usage(1_500).expect("记录用量失败"),
            (1_500, 1_500)
        );
        assert_eq!(
            store.record_token_usage(500).expect("记录用量失败"),
            (2_000, 2_000)
        );
        assert_eq!(
            store.load_usage_totals().expect("读取用量失败"),
            (2_000, 2_000)
        );

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
