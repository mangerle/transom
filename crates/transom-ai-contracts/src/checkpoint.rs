//! 按对话轮次的文件备份检查点（Checkpoint）与一键回滚记录。
//!
//! 设计原理（Why）：
//! - 将文件写入/编辑时创建的 `.bak` 备份与对话树 `SessionEntry` ID 绑定；
//! - 用户点击“回退到此轮之前”时，可按逆序精确还原该轮及之后修改过的本机与远端文件，
//!   同时将对话树 `active_leaf_id` 切回该轮之前，实现状态与文件的双重回滚。

use serde::{Deserialize, Serialize};

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

    /// 选择从 `entry_id` 起（含该轮）及之后的所有轮次，并从清单中暂时移出。
    ///
    /// 调用方必须在全部文件还原成功后再持久化清单；失败记录需重新登记，
    /// 从而保证失败部分仍可重试。保留完整的轮次分组，使回滚顺序与原始写入一致。
    pub fn take_rollback_turns(&mut self, entry_id: &str) -> Vec<TurnCheckpoint> {
        let Some(pos) = self.turns.iter().position(|t| t.entry_id == entry_id) else {
            return Vec::new();
        };
        self.turns.drain(pos..).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rollback_selection_preserves_turn_grouping() {
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
        let selected = manifest.take_rollback_turns("u1");
        assert_eq!(selected.len(), 2);
        assert_eq!(selected[0].entry_id, "u1");
        assert_eq!(selected[1].entry_id, "u2");
        assert_eq!(selected[0].backups[0].path, "/tmp/a.conf");
        assert_eq!(selected[1].backups[0].path, "/etc/nginx/b.conf");
        assert!(manifest.turns.is_empty());
    }
}
