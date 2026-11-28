//! AI 会话索引（`index.json`）与树状 JSONL（`sessions/<id>.jsonl`）读写管理。

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use transom_ai_contracts::{AiError, ConversationSummary, SessionEntry, SessionScope, SessionTree};

use super::{AiStore, resolve_id_file_path};
use crate::file_io::write_atomic;
use transom_platform::private_fs;

/// 单个会话索引文件的最大体积。
const MAX_INDEX_BYTES: u64 = 8 * 1024 * 1024;
/// 单个会话 JSONL 文件的最大体积。
const MAX_SESSION_JSONL_BYTES: u64 = 32 * 1024 * 1024;
/// 单条会话记录的最大体积，避免一次超长工具输出撑爆整份会话。
const MAX_SESSION_ENTRY_BYTES: usize = 4 * 1024 * 1024;

impl AiStore {
    /// 读取对话索引列表的只读共享快照（缓存命中时仅拷贝 `Arc` 指针，零整表深拷贝）。
    pub fn load_index_arc(&self) -> Result<Arc<Vec<ConversationSummary>>, AiError> {
        let _guard = self.index_lock.lock();
        self.read_index_unlocked()
    }

    /// 读取对话索引列表，按 `updated_at` 降序排列（优先命中进程内索引缓存）。
    pub fn load_index(&self) -> Result<Vec<ConversationSummary>, AiError> {
        Ok((*self.load_index_arc()?).clone())
    }

    fn read_index_unlocked(&self) -> Result<Arc<Vec<ConversationSummary>>, AiError> {
        if let Some(cached) = self.index_cache.lock().as_ref() {
            return Ok(Arc::clone(cached));
        }
        let path = self.root.join("index.json");
        let list = if !path.exists() {
            Vec::new()
        } else {
            let metadata = fs::metadata(&path).map_err(|err| AiError::StorageFailed {
                target: path.display().to_string(),
                reason: err.to_string(),
            })?;
            if metadata.len() > MAX_INDEX_BYTES {
                return Err(AiError::StorageFailed {
                    target: path.display().to_string(),
                    reason: format!(
                        "AI 会话索引超过体积上限（实际 {} 字节，上限 {MAX_INDEX_BYTES} 字节）",
                        metadata.len()
                    ),
                });
            }
            let text = fs::read_to_string(&path).map_err(|err| AiError::StorageFailed {
                target: path.display().to_string(),
                reason: err.to_string(),
            })?;
            let mut parsed: Vec<ConversationSummary> =
                serde_json::from_str(&text).map_err(|err| AiError::StorageFailed {
                    target: path.display().to_string(),
                    reason: err.to_string(),
                })?;
            parsed.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
            parsed
        };
        let arc = Arc::new(list);
        *self.index_cache.lock() = Some(Arc::clone(&arc));
        Ok(arc)
    }

    /// 在进程内互斥锁保护下执行 `index.json` 的内存更新与紧凑 JSON 原子写回。
    fn update_index(
        &self,
        mutator: impl FnOnce(&mut Vec<ConversationSummary>) -> bool,
    ) -> Result<(), AiError> {
        let _guard = self.index_lock.lock();
        let current = self.read_index_unlocked()?;
        let mut list = (*current).clone();
        if !mutator(&mut list) {
            return Ok(());
        }
        list.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        let path = self.root.join("index.json");
        let payload = serde_json::to_string(&list).map_err(|err| AiError::StorageFailed {
            target: path.display().to_string(),
            reason: err.to_string(),
        })?;
        write_atomic(&path, payload.as_bytes()).map_err(|err| AiError::StorageFailed {
            target: path.display().to_string(),
            reason: err.to_string(),
        })?;
        *self.index_cache.lock() = Some(Arc::new(list));
        Ok(())
    }

    /// 返回全局全部对话摘要列表（按更新时间倒序）。
    pub fn list_all_conversations(&self) -> Result<Vec<ConversationSummary>, AiError> {
        self.load_index()
    }

    /// 按作用域筛选对话摘要列表（仅克隆命中项）。
    pub fn list_conversations_for_scope(
        &self,
        scope: &SessionScope,
    ) -> Result<Vec<ConversationSummary>, AiError> {
        let index = self.load_index_arc()?;
        Ok(index
            .iter()
            .filter(|item| &item.scope == scope)
            .cloned()
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
        let active_leaf_id = self
            .load_index_arc()?
            .iter()
            .find(|summary| summary.id == conversation_id)
            .and_then(|summary| summary.active_leaf_id.clone());
        let metadata = fs::metadata(&path).map_err(|err| AiError::StorageFailed {
            target: path.display().to_string(),
            reason: err.to_string(),
        })?;
        if metadata.len() > MAX_SESSION_JSONL_BYTES {
            return Err(AiError::StorageFailed {
                target: path.display().to_string(),
                reason: format!(
                    "会话文件超过体积上限（实际 {} 字节，上限 {MAX_SESSION_JSONL_BYTES} 字节）",
                    metadata.len()
                ),
            });
        }
        let text = fs::read_to_string(&path).map_err(|err| AiError::StorageFailed {
            target: path.display().to_string(),
            reason: err.to_string(),
        })?;
        SessionTree::from_jsonl_with_active_leaf(&text, active_leaf_id.as_deref())
    }

    /// 预解析并确保对话 JSONL 的沙箱路径及私有目录就绪，供长运行期复用以避免重复 `canonicalize`。
    pub fn prepare_session_path(&self, conversation_id: &str) -> Result<PathBuf, AiError> {
        let path = self.session_file_path(conversation_id)?;
        if let Some(parent) = path.parent() {
            private_fs::ensure_private_dir(parent).map_err(|err| AiError::StorageFailed {
                target: parent.display().to_string(),
                reason: err.to_string(),
            })?;
        }
        Ok(path)
    }

    /// 向已校验的对话 JSONL 末尾批量追加多条记录（按 `conversation_id` 分段加锁并复用单次文件句柄）。
    pub fn append_session_entries_prepared(
        &self,
        conversation_id: &str,
        path: &Path,
        entries: &[SessionEntry],
    ) -> Result<(), AiError> {
        if entries.is_empty() {
            return Ok(());
        }
        let mut lines = Vec::with_capacity(entries.len());
        let mut added_bytes = 0u64;
        for entry in entries {
            let line = SessionTree::encode_entry(entry)?;
            if line.len() > MAX_SESSION_ENTRY_BYTES {
                return Err(AiError::StorageFailed {
                    target: path.display().to_string(),
                    reason: format!(
                        "单条会话记录超过体积上限（实际 {} 字节，上限 {MAX_SESSION_ENTRY_BYTES} 字节）",
                        line.len()
                    ),
                });
            }
            added_bytes = added_bytes
                .saturating_add(line.len() as u64)
                .saturating_add(1);
            lines.push(line);
        }

        let _guard = self.session_append_lock(conversation_id).lock();
        let existing_bytes = match fs::metadata(path) {
            Ok(metadata) => metadata.len(),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => 0,
            Err(err) => {
                return Err(AiError::StorageFailed {
                    target: path.display().to_string(),
                    reason: err.to_string(),
                });
            }
        };
        let next_bytes = existing_bytes.saturating_add(added_bytes);
        if next_bytes > MAX_SESSION_JSONL_BYTES {
            return Err(AiError::StorageFailed {
                target: path.display().to_string(),
                reason: format!(
                    "会话文件写入后将超过体积上限（预计 {next_bytes} 字节，上限 {MAX_SESSION_JSONL_BYTES} 字节）"
                ),
            });
        }
        let mut file =
            private_fs::open_private_append(path).map_err(|err| AiError::StorageFailed {
                target: path.display().to_string(),
                reason: err.to_string(),
            })?;
        for line in lines {
            writeln!(file, "{line}").map_err(|err| AiError::StorageFailed {
                target: path.display().to_string(),
                reason: err.to_string(),
            })?;
        }
        Ok(())
    }

    /// 向指定对话的 JSONL 末尾追加一条记录。
    pub fn append_session_entry(
        &self,
        conversation_id: &str,
        entry: &SessionEntry,
    ) -> Result<(), AiError> {
        let path = self.prepare_session_path(conversation_id)?;
        self.append_session_entries_prepared(conversation_id, &path, std::slice::from_ref(entry))
    }

    fn session_file_path(&self, conversation_id: &str) -> Result<PathBuf, AiError> {
        let sessions_dir = self.root.join("sessions");
        resolve_id_file_path(&sessions_dir, conversation_id, "jsonl")
    }
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
