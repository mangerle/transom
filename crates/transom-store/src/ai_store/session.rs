//! AI 会话索引（`index.json`）与树状 JSONL（`sessions/<id>.jsonl`）读写管理。

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use transom_ai::{
    AiError, ConversationSummary, SessionEntry, SessionScope, SessionTree, resolve_sandbox_path,
};

use super::{AiStore, sanitize_id_filename};
use crate::file_io::write_atomic;

impl AiStore {
    /// 读取对话索引列表，按 `updated_at` 降序排列（优先命中进程内索引缓存）。
    pub fn load_index(&self) -> Result<Vec<ConversationSummary>, AiError> {
        let _guard = self.index_lock.lock();
        self.read_index_unlocked()
    }

    fn read_index_unlocked(&self) -> Result<Vec<ConversationSummary>, AiError> {
        if let Some(cached) = self.index_cache.lock().as_ref() {
            return Ok(cached.clone());
        }
        let path = self.root.join("index.json");
        let list = if !path.exists() {
            Vec::new()
        } else {
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
        *self.index_cache.lock() = Some(list.clone());
        Ok(list)
    }

    /// 在进程内互斥锁保护下执行 `index.json` 的内存更新与紧凑 JSON 原子写回。
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
        let payload = serde_json::to_string(&list).map_err(|err| AiError::StorageFailed {
            target: path.display().to_string(),
            reason: err.to_string(),
        })?;
        write_atomic(&path, payload.as_bytes()).map_err(|err| AiError::StorageFailed {
            target: path.display().to_string(),
            reason: err.to_string(),
        })?;
        *self.index_cache.lock() = Some(list);
        Ok(())
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

    fn session_file_path(&self, conversation_id: &str) -> Result<PathBuf, AiError> {
        let sessions_dir = self.root.join("sessions");
        let file_name = sanitize_id_filename(conversation_id, "jsonl");
        resolve_sandbox_path(&sessions_dir, Path::new(&file_name))
    }
}

#[cfg(test)]
mod tests {
    use super::super::AiStore;
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
}
