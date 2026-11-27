//! AI 检查点清单的持久化、并发保护与原子写入。
//!
//! 设计原理（Why）：
//! - 检查点决定文件能否安全回滚，必须使用与其它私有配置一致的原子写入，不能直接覆盖；
//! - 读改写流程由 `checkpoint_lock` 串行化，避免多个工具同时备份文件时互相覆盖记录；
//! - 清单读取设置体积上限，防止损坏或恶意膨胀文件拖垮主流程。

use std::fs;
use std::path::{Path, PathBuf};

use transom_ai::{AiError, CheckpointManifest, FileBackupRecord};
use transom_core::AppError;

use super::{AiStore, sanitize_id_filename};
use crate::file_io::write_atomic;

/// 单个对话检查点清单允许的最大体积。
pub const MAX_CHECKPOINT_MANIFEST_BYTES: u64 = 8 * 1024 * 1024;

impl AiStore {
    /// 读取指定对话的检查点清单；文件不存在时返回空清单。
    pub fn load_checkpoint_manifest(
        &self,
        conversation_id: &str,
    ) -> Result<CheckpointManifest, AiError> {
        let _guard = self.checkpoint_lock.lock();
        self.load_checkpoint_manifest_unlocked(conversation_id)
    }

    /// 原子保存完整检查点清单。
    pub fn save_checkpoint_manifest(&self, manifest: &CheckpointManifest) -> Result<(), AiError> {
        let _guard = self.checkpoint_lock.lock();
        self.save_checkpoint_manifest_unlocked(manifest)
    }

    /// 在同一把锁内读取、追加单条备份记录并原子写回。
    pub fn append_checkpoint_record(
        &self,
        conversation_id: &str,
        entry_id: &str,
        record: FileBackupRecord,
    ) -> Result<(), AiError> {
        let _guard = self.checkpoint_lock.lock();
        let mut manifest = self.load_checkpoint_manifest_unlocked(conversation_id)?;
        manifest.record_backup(entry_id, record);
        self.save_checkpoint_manifest_unlocked(&manifest)
    }

    fn load_checkpoint_manifest_unlocked(
        &self,
        conversation_id: &str,
    ) -> Result<CheckpointManifest, AiError> {
        let path = self.checkpoint_manifest_path(conversation_id)?;
        if !path.exists() {
            return Ok(CheckpointManifest {
                conversation_id: conversation_id.to_owned(),
                turns: Vec::new(),
            });
        }
        let metadata = fs::metadata(&path).map_err(|err| AiError::StorageFailed {
            target: path.display().to_string(),
            reason: err.to_string(),
        })?;
        if metadata.len() > MAX_CHECKPOINT_MANIFEST_BYTES {
            return Err(AiError::StorageFailed {
                target: path.display().to_string(),
                reason: format!(
                    "检查点清单超过体积上限（实际 {} 字节，上限 {} 字节）",
                    metadata.len(),
                    MAX_CHECKPOINT_MANIFEST_BYTES
                ),
            });
        }
        let raw = fs::read_to_string(&path).map_err(|err| AiError::StorageFailed {
            target: path.display().to_string(),
            reason: err.to_string(),
        })?;
        serde_json::from_str(&raw).map_err(|err| AiError::StorageFailed {
            target: path.display().to_string(),
            reason: err.to_string(),
        })
    }

    fn save_checkpoint_manifest_unlocked(
        &self,
        manifest: &CheckpointManifest,
    ) -> Result<(), AiError> {
        let path = self.checkpoint_manifest_path(&manifest.conversation_id)?;
        let payload =
            serde_json::to_vec_pretty(manifest).map_err(|err| AiError::StorageFailed {
                target: path.display().to_string(),
                reason: err.to_string(),
            })?;
        write_atomic(&path, &payload).map_err(|err| map_storage_error(&path, err))
    }

    fn checkpoint_manifest_path(&self, conversation_id: &str) -> Result<PathBuf, AiError> {
        let checkpoint_dir = self.root.join("checkpoints");
        let file_name = sanitize_id_filename(conversation_id, "json");
        transom_ai::resolve_sandbox_path(&checkpoint_dir, Path::new(&file_name))
    }
}

fn map_storage_error(path: &Path, error: AppError) -> AiError {
    AiError::StorageFailed {
        target: path.display().to_string(),
        reason: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::AiStore;
    use transom_ai::{CheckpointManifest, FileBackupRecord};

    #[test]
    fn checkpoint_manifest_roundtrips_and_appends_in_one_transaction() {
        let dir = tempfile::tempdir().expect("创建临时目录失败");
        let store = AiStore::with_root(dir.path().to_path_buf());
        let record = FileBackupRecord {
            target: "local".to_owned(),
            path: "/tmp/app.conf".to_owned(),
            backup_path: Some("/tmp/app.conf.bak".to_owned()),
            existed_before: true,
        };
        store
            .append_checkpoint_record("conv-1", "turn-1", record.clone())
            .expect("追加检查点失败");
        store
            .append_checkpoint_record("conv-1", "turn-1", record.clone())
            .expect("追加第二条检查点失败");

        let manifest = store
            .load_checkpoint_manifest("conv-1")
            .expect("读取检查点失败");
        assert_eq!(manifest.turns.len(), 1);
        assert_eq!(manifest.turns[0].backups.len(), 2);

        let empty = CheckpointManifest {
            conversation_id: "conv-2".to_owned(),
            turns: Vec::new(),
        };
        store
            .save_checkpoint_manifest(&empty)
            .expect("保存空检查点失败");
        assert!(
            store
                .load_checkpoint_manifest("conv-2")
                .expect("读取空检查点失败")
                .turns
                .is_empty()
        );
    }
}
