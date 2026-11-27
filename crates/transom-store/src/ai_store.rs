//! AI 助手本地持久化：供应商配置、树状 JSONL 会话、三层记忆与审计日志路径。
//!
//! 设计原理（Why）：
//! - 配置文件 `ai/config.json` 仅保存供应商端点与模型能力，API Key 统一走 [`SecretStore`] 钥匙串；
//! - 会话采用轻量 `index.json` + 追加写 `sessions/<id>.jsonl`，兼顾侧栏秒开与追加低 IO；
//! - 所有动态文件名均经过 [`resolve_sandbox_path`] 规范化校验，杜绝路径穿越。

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use transom_ai::{
    AiAuditEntry, AiError, AiSettings, ConversationSummary, MemorySnapshot, SessionEntry,
    SessionScope, SessionTree, read_memory_file_limited, resolve_sandbox_path,
};
use transom_core::{AppError, SecretKey, SecretStore};
use zeroize::Zeroizing;

use crate::file_io::write_atomic;
use crate::paths::config_dir;

/// AI 助手本地数据目录管理器。
#[derive(Clone, Debug)]
pub struct AiStore {
    root: PathBuf,
    index_lock: Arc<Mutex<()>>,
    usage_lock: Arc<Mutex<()>>,
}

impl Default for AiStore {
    fn default() -> Self {
        Self::new()
    }
}

impl AiStore {
    /// 使用平台默认配置目录下的 `ai/` 子目录构造存储实例。
    pub fn new() -> Self {
        Self::with_root(config_dir().join("ai"))
    }

    /// 使用指定根目录构造存储实例（供单元测试隔离使用）。
    pub fn with_root(root: PathBuf) -> Self {
        Self {
            root,
            index_lock: Arc::new(Mutex::new(())),
            usage_lock: Arc::new(Mutex::new(())),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// 读取 AI 配置（不存在时返回默认值）。
    pub fn load_settings(&self) -> Result<AiSettings, AiError> {
        let path = self.root.join("config.json");
        if !path.exists() {
            return Ok(AiSettings::default());
        }
        let text = fs::read_to_string(&path).map_err(|err| AiError::StorageFailed {
            target: path.display().to_string(),
            reason: err.to_string(),
        })?;
        serde_json::from_str(&text).map_err(|err| AiError::StorageFailed {
            target: path.display().to_string(),
            reason: err.to_string(),
        })
    }

    /// 原子写入 AI 配置（不含明文 API Key）。
    pub fn save_settings(&self, settings: &AiSettings) -> Result<(), AiError> {
        let path = self.root.join("config.json");
        let payload =
            serde_json::to_string_pretty(settings).map_err(|err| AiError::StorageFailed {
                target: path.display().to_string(),
                reason: err.to_string(),
            })?;
        write_atomic(&path, payload.as_bytes()).map_err(|err| AiError::StorageFailed {
            target: path.display().to_string(),
            reason: err.to_string(),
        })
    }

    /// 从 OS Keyring 读取指定供应商的 API Key。
    pub fn load_provider_api_key(
        &self,
        secrets: &dyn SecretStore,
        provider_id: &str,
    ) -> Result<Option<Zeroizing<String>>, AppError> {
        secrets.get(&SecretKey::ai_provider_key(provider_id))
    }

    /// 将指定供应商的 API Key 写入 OS Keyring。
    pub fn save_provider_api_key(
        &self,
        secrets: &dyn SecretStore,
        provider_id: &str,
        api_key: &str,
    ) -> Result<(), AppError> {
        let key = SecretKey::ai_provider_key(provider_id);
        if api_key.trim().is_empty() {
            secrets.delete(&key)
        } else {
            secrets.set(&key, api_key.trim())
        }
    }

    /// 读取对话索引列表，按 `updated_at` 降序排列。
    pub fn load_index(&self) -> Result<Vec<ConversationSummary>, AiError> {
        let _guard = self.index_lock.lock();
        self.read_index_unlocked()
    }

    fn read_index_unlocked(&self) -> Result<Vec<ConversationSummary>, AiError> {
        let path = self.root.join("index.json");
        if !path.exists() {
            return Ok(Vec::new());
        }
        let text = fs::read_to_string(&path).map_err(|err| AiError::StorageFailed {
            target: path.display().to_string(),
            reason: err.to_string(),
        })?;
        let mut list: Vec<ConversationSummary> =
            serde_json::from_str(&text).map_err(|err| AiError::StorageFailed {
                target: path.display().to_string(),
                reason: err.to_string(),
            })?;
        list.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        Ok(list)
    }

    /// 在进程内互斥锁保护下执行 `index.json` 的读-改-原子写。
    fn update_index(
        &self,
        mutator: impl FnOnce(&mut Vec<ConversationSummary>) -> bool,
    ) -> Result<(), AiError> {
        let _guard = self.index_lock.lock();
        let mut list = self.read_index_unlocked()?;
        if !mutator(&mut list) {
            return Ok(());
        }
        list.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        let path = self.root.join("index.json");
        let payload =
            serde_json::to_string_pretty(&list).map_err(|err| AiError::StorageFailed {
                target: path.display().to_string(),
                reason: err.to_string(),
            })?;
        write_atomic(&path, payload.as_bytes()).map_err(|err| AiError::StorageFailed {
            target: path.display().to_string(),
            reason: err.to_string(),
        })
    }

    /// 返回全局全部对话摘要列表（按更新时间倒序）。
    pub fn list_all_conversations(&self) -> Result<Vec<ConversationSummary>, AiError> {
        self.load_index()
    }

    /// 按作用域筛选对话摘要列表。
    pub fn list_conversations_for_scope(
        &self,
        scope: &SessionScope,
    ) -> Result<Vec<ConversationSummary>, AiError> {
        Ok(self
            .load_index()?
            .into_iter()
            .filter(|item| &item.scope == scope)
            .collect())
    }

    /// 插入或更新单个对话摘要到索引文件中。
    pub fn upsert_conversation(&self, summary: ConversationSummary) -> Result<(), AiError> {
        self.update_index(|list| {
            if let Some(existing) = list.iter_mut().find(|item| item.id == summary.id) {
                *existing = summary;
            } else {
                list.push(summary);
            }
            true
        })
    }

    /// 原子更新指定对话的活动叶节点 ID 与更新时间。
    pub fn update_conversation_leaf(
        &self,
        conversation_id: &str,
        active_leaf_id: Option<String>,
        updated_at: String,
    ) -> Result<(), AiError> {
        self.update_index(|list| {
            let Some(item) = list.iter_mut().find(|item| item.id == conversation_id) else {
                return false;
            };
            item.active_leaf_id = active_leaf_id;
            item.updated_at = updated_at;
            true
        })
    }

    /// 切换指定对话的归档状态并写回 `index.json`。
    pub fn set_conversation_archived(
        &self,
        conversation_id: &str,
        archived: bool,
    ) -> Result<(), AiError> {
        self.update_index(|list| {
            let Some(item) = list.iter_mut().find(|item| item.id == conversation_id) else {
                return false;
            };
            item.archived = archived;
            true
        })
    }

    /// 重命名指定对话标题并写回 `index.json`。
    pub fn rename_conversation(
        &self,
        conversation_id: &str,
        new_title: &str,
    ) -> Result<(), AiError> {
        self.update_index(|list| {
            let Some(item) = list.iter_mut().find(|item| item.id == conversation_id) else {
                return false;
            };
            item.title = new_title.to_owned();
            true
        })
    }

    /// 删除指定对话索引及对应的 JSONL 会话文件。
    pub fn delete_conversation(&self, conversation_id: &str) -> Result<(), AiError> {
        self.update_index(|list| {
            let before = list.len();
            list.retain(|item| item.id != conversation_id);
            list.len() != before
        })?;

        let session_path = self.session_file_path(conversation_id)?;
        if session_path.exists() {
            fs::remove_file(&session_path).map_err(|err| AiError::StorageFailed {
                target: session_path.display().to_string(),
                reason: err.to_string(),
            })?;
        }
        Ok(())
    }

    /// 读取指定对话的树状会话记录。
    pub fn load_session_tree(&self, conversation_id: &str) -> Result<SessionTree, AiError> {
        let path = self.session_file_path(conversation_id)?;
        if !path.exists() {
            return Ok(SessionTree::default());
        }
        let text = fs::read_to_string(&path).map_err(|err| AiError::StorageFailed {
            target: path.display().to_string(),
            reason: err.to_string(),
        })?;
        SessionTree::from_jsonl(&text)
    }

    /// 向指定对话的 JSONL 末尾追加一条记录。
    pub fn append_session_entry(
        &self,
        conversation_id: &str,
        entry: &SessionEntry,
    ) -> Result<(), AiError> {
        let path = self.session_file_path(conversation_id)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|err| AiError::StorageFailed {
                target: parent.display().to_string(),
                reason: err.to_string(),
            })?;
        }
        let line = SessionTree::encode_entry(entry)?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|err| AiError::StorageFailed {
                target: path.display().to_string(),
                reason: err.to_string(),
            })?;
        writeln!(file, "{line}").map_err(|err| AiError::StorageFailed {
            target: path.display().to_string(),
            reason: err.to_string(),
        })
    }

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

    fn session_file_path(&self, conversation_id: &str) -> Result<PathBuf, AiError> {
        let sessions_dir = self.root.join("sessions");
        let file_name = sanitize_id_filename(conversation_id, "jsonl");
        resolve_sandbox_path(&sessions_dir, Path::new(&file_name))
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

fn sanitize_id_filename(raw_id: &str, ext: &str) -> String {
    let cleaned: String = raw_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let stem = if cleaned.is_empty() {
        "default"
    } else {
        cleaned.as_str()
    };
    format!("{stem}.{ext}")
}

#[cfg(test)]
mod tests {
    use super::AiStore;
    use transom_ai::{ChatRole, ConversationSummary, SessionEntry, SessionEntryKind, SessionScope};

    #[test]
    fn session_index_and_jsonl_roundtrip() {
        let dir = tempfile::tempdir().expect("创建临时目录失败");
        let store = AiStore::with_root(dir.path().to_path_buf());

        let conv_id = "conv-test-1";
        store
            .upsert_conversation(ConversationSummary {
                id: conv_id.to_owned(),
                title: "巡检主机".to_owned(),
                scope: SessionScope::Host("host-a".to_owned()),
                model_id: "m1".to_owned(),
                effort: None,
                updated_at: "2026-11-26T10:00:00Z".to_owned(),
                active_leaf_id: None,
                archived: false,
            })
            .expect("写入索引失败");

        let entry = SessionEntry::new(
            None,
            SessionEntryKind::Message {
                role: ChatRole::User,
                text: "看看负载".to_owned(),
                images: vec![],
                model: None,
                tokens_per_sec: None,
                usage: None,
            },
        );
        store
            .append_session_entry(conv_id, &entry)
            .expect("追加 JSONL 失败");

        let list = store
            .list_conversations_for_scope(&SessionScope::Host("host-a".to_owned()))
            .expect("查询索引失败");
        assert_eq!(list.len(), 1);
        assert!(!list[0].archived);

        store
            .set_conversation_archived(conv_id, true)
            .expect("归档失败");
        assert!(store.list_all_conversations().unwrap()[0].archived);
        store
            .set_conversation_archived(conv_id, false)
            .expect("取消归档失败");
        assert!(!store.list_all_conversations().unwrap()[0].archived);

        store
            .rename_conversation(conv_id, "生产巡检记录")
            .expect("重命名失败");
        assert_eq!(store.list_all_conversations().unwrap()[0].title, "生产巡检记录");

        let tree = store.load_session_tree(conv_id).expect("加载会话树失败");
        assert_eq!(tree.all_entries().len(), 1);
    }

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

        let mut totals = super::AiUsageTotals {
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
