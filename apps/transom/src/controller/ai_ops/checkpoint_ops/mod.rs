//! 按对话轮次的 Checkpoint 备份、事务式回滚与文件还原。
//!
//! 设计原理（Why）：
//! - 备份是写文件的前置条件：无法确认“文件原本不存在”或无法保存旧内容时，写操作必须失败；
//! - 回滚采用“选择、还原、提交”三阶段，失败记录重新登记，只有全部文件恢复成功才回退对话树；
//! - 清单持久化下沉到 `AiStore`，由存储层提供锁、原子写与体积上限。

mod backup;
mod restore;
mod rollback;

use std::path::{Path, PathBuf};

use transom_ai::ToolError;
use transom_store::AiStore;

use super::app_bridge::AppBridgeDeps;

pub(super) use backup::{record_local_backup, record_remote_backup};
pub(super) use restore::spawn_revert_remote_file;
pub(super) use rollback::spawn_rollback_checkpoint;

const MAX_CHECKPOINT_BACKUP_BYTES: usize = 2 * 1024 * 1024;

pub(super) fn store_ai_root(store: &AiStore) -> PathBuf {
    store
        .custom_skills_dir()
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| store.custom_skills_dir())
}

fn checkpoint_context(deps: &AppBridgeDeps) -> Option<(String, String)> {
    let conversation_id = deps.conversation_id.clone()?;
    let entry_id = deps.current_turn_entry_id.read().clone()?;
    Some((conversation_id, entry_id))
}

fn checkpoint_blob_dir(deps: &AppBridgeDeps) -> PathBuf {
    store_ai_root(&deps.ai_store)
        .join("checkpoints")
        .join("blobs")
}

fn checkpoint_error(reason: String) -> ToolError {
    ToolError::ExecutionFailed {
        tool: "checkpoint",
        reason,
    }
}
