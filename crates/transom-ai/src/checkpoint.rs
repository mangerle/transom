//! 按对话轮次的文件备份检查点（Checkpoint）与一键回滚记录。
//!
//! 设计原理（Why）：
//! - 将文件写入/编辑时创建的 `.bak` 备份与对话树 `SessionEntry` ID 绑定；
//! - 用户点击“回退到此轮之前”时，可按逆序精确还原该轮及之后修改过的本机与远端文件，
//!   同时将对话树 `active_leaf_id` 切回该轮之前，实现状态与文件的双重回滚。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::AiError;

/// 单次文件变更的备份记录。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileBackupRecord {
    /// 目标标识：`"local"` 或远端服务器 `session_id` / 名称。
    pub target: String,
    /// 被修改文件的原始绝对路径。
    pub path: String,
    /// 备份文件路径；若写入前文件不存在则为 `None`（回滚时删除新文件）。
    #[serde(default)]
    pub backup_path: Option<String>,
    /// 写入前文件是否已存在。
    #[serde(default)]
    pub existed_before: bool,
}

/// 绑定到某一用户轮次的检查点快照。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnCheckpoint {
    pub entry_id: String,
    #[serde(default)]
    pub backups: Vec<FileBackupRecord>,
}

/// 单会话的检查点持久化文件。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointManifest {
    pub conversation_id: String,
    #[serde(default)]
    pub turns: Vec<TurnCheckpoint>,
}

impl CheckpointManifest {
    /// 获取检查点文件存储路径（`~/.transom/ai/checkpoints/<conv_id>.json`）。
    pub fn file_path(base_dir: &std::path::Path, conversation_id: &str) -> PathBuf {
        let safe_id: String = conversation_id
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || matches!(c, '-' | '_') {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        base_dir.join("checkpoints").join(format!("{safe_id}.json"))
    }

    /// 从磁盘加载检查点清单（不存在则返回空清单）。
    pub fn load(base_dir: &std::path::Path, conversation_id: &str) -> Result<Self, AiError> {
        let path = Self::file_path(base_dir, conversation_id);
        if !path.exists() {
            return Ok(Self {
                conversation_id: conversation_id.to_owned(),
                turns: Vec::new(),
            });
        }
        let raw = std::fs::read_to_string(&path).map_err(|err| AiError::StorageFailed {
            target: path.display().to_string(),
            reason: err.to_string(),
        })?;
        serde_json::from_str(&raw).map_err(|err| AiError::StorageFailed {
            target: path.display().to_string(),
            reason: err.to_string(),
        })
    }

    /// 持久化写回磁盘。
    pub fn save(&self, base_dir: &std::path::Path) -> Result<(), AiError> {
        let path = Self::file_path(base_dir, &self.conversation_id);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|err| AiError::StorageFailed {
                target: parent.display().to_string(),
                reason: err.to_string(),
            })?;
        }
        let json = serde_json::to_string_pretty(self).map_err(|err| AiError::StorageFailed {
            target: path.display().to_string(),
            reason: err.to_string(),
        })?;
        std::fs::write(&path, json).map_err(|err| AiError::StorageFailed {
            target: path.display().to_string(),
            reason: err.to_string(),
        })
    }

    /// 向指定轮次追加一条文件备份记录。
    pub fn record_backup(&mut self, entry_id: &str, record: FileBackupRecord) {
        if let Some(turn) = self.turns.iter_mut().find(|t| t.entry_id == entry_id) {
            turn.backups.push(record);
        } else {
            self.turns.push(TurnCheckpoint {
                entry_id: entry_id.to_owned(),
                backups: vec![record],
            });
        }
    }

    /// 收集从 `entry_id` 起（含该轮）及之后所有轮次的备份记录，按逆序返回供回滚还原，并截断清单。
    pub fn drain_rollback_records(&mut self, entry_id: &str) -> Vec<FileBackupRecord> {
        let Some(pos) = self.turns.iter().position(|t| t.entry_id == entry_id) else {
            return Vec::new();
        };
        let removed: Vec<TurnCheckpoint> = self.turns.drain(pos..).collect();
        removed
            .into_iter()
            .rev()
            .flat_map(|t| t.backups.into_iter().rev())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rollback_drains_records_in_reverse_order() {
        let mut manifest = CheckpointManifest {
            conversation_id: "c1".into(),
            turns: Vec::new(),
        };
        manifest.record_backup(
            "u1",
            FileBackupRecord {
                target: "local".into(),
                path: "/tmp/a.conf".into(),
                backup_path: Some("/tmp/a.conf.bak1".into()),
                existed_before: true,
            },
        );
        manifest.record_backup(
            "u2",
            FileBackupRecord {
                target: "prod-01".into(),
                path: "/etc/nginx/b.conf".into(),
                backup_path: Some("/etc/nginx/b.conf.bak2".into()),
                existed_before: true,
            },
        );
        let drained = manifest.drain_rollback_records("u1");
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].path, "/etc/nginx/b.conf");
        assert_eq!(drained[1].path, "/tmp/a.conf");
        assert!(manifest.turns.is_empty());
    }
}
