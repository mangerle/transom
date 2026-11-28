//! 事务式检查点回滚与失败记录保留。

use std::collections::HashSet;
use std::sync::Arc;

use tokio_util::sync::CancellationToken;
use transom_ai::{ToolIoContext, TurnCheckpoint};
use transom_core::{AiAppEvent, AppEvent};
use transom_net::{SftpClient, SshRemote};
use transom_store::AiStore;

use super::restore::{restore_local, restore_remote};
use crate::controller::state::{ControllerCtx, ControllerState};

/// 执行事务式按轮次回滚。
pub(in crate::controller::ai_ops) fn spawn_rollback_checkpoint(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    conversation_id: String,
    entry_id: String,
) {
    let store = ctx.ai_store.clone();
    let event_tx = ctx.event_tx.clone();
    let sessions: Vec<(String, SshRemote, Option<SftpClient>)> = state
        .sessions
        .values()
        .filter_map(|m| {
            m.remote
                .clone()
                .map(|r| (m.connection_id.to_string(), r, m.sftp_client.clone()))
        })
        .collect();
    let session_catalog = ctx.session_store.load().unwrap_or_default();

    state.ai.run_set.spawn(async move {
        let io = ToolIoContext::new(
            super::REMOTE_RESTORE_TIMEOUT,
            CancellationToken::new(),
        );
        let load_store = store.clone();
        let load_conversation = conversation_id.clone();
        let loaded = tokio::task::spawn_blocking(move || {
            load_store.load_checkpoint_manifest(&load_conversation)
        })
        .await;
        let mut manifest = match loaded {
            Ok(Ok(manifest)) => manifest,
            Ok(Err(err)) => {
                emit_rollback_result(
                    &event_tx,
                    &conversation_id,
                    0,
                    format!("读取检查点失败，已取消回退: {err}"),
                )
                .await;
                return;
            }
            Err(err) => {
                emit_rollback_result(
                    &event_tx,
                    &conversation_id,
                    0,
                    format!("检查点读取任务异常，已取消回退: {err}"),
                )
                .await;
                return;
            }
        };

        let rollback_turns = manifest.take_rollback_turns(&entry_id);
        let mut failed_records = HashSet::with_capacity(rollback_turns.len());
        let mut restored_files = 0usize;
        let mut failed_files = 0usize;

        for (turn_idx, turn) in rollback_turns.iter().enumerate().rev() {
            for (record_idx, record) in turn.backups.iter().enumerate().rev() {
                let result = if record.target == "local" {
                    restore_local(record).await
                } else {
                    match find_remote_session(&sessions, &session_catalog, &record.target) {
                        Some((remote, sftp)) => {
                            restore_remote(record, remote, sftp.clone(), &io).await
                        }
                        None => Err(format!(
                            "未找到主机「{}」的活跃 SSH 会话，无法还原 {}",
                            record.target, record.path
                        )),
                    }
                };
                match result {
                    Ok(()) => restored_files = restored_files.saturating_add(1),
                    Err(err) => {
                        failed_files = failed_files.saturating_add(1);
                        failed_records.insert((turn_idx, record_idx));
                        log::warn!("检查点文件还原失败: {err}");
                    }
                }
            }
        }

        requeue_failed_records(&mut manifest, &rollback_turns, &failed_records);

        let mut tree_error = None;
        let mut manifest_save_error = None;
        if failed_files == 0 {
            match rollback_conversation_leaf(store.clone(), &conversation_id, &entry_id).await {
                Ok(()) => {
                    let save_store = store.clone();
                    let save_result = tokio::task::spawn_blocking(move || {
                        save_store.save_checkpoint_manifest(&manifest)
                    })
                    .await;
                    match save_result {
                        Ok(Ok(())) => {}
                        Ok(Err(err)) => manifest_save_error = Some(err.to_string()),
                        Err(err) => manifest_save_error = Some(err.to_string()),
                    }
                }
                Err(err) => tree_error = Some(err),
            }
        } else {
            let save_store = store.clone();
            let save_result = tokio::task::spawn_blocking(move || {
                save_store.save_checkpoint_manifest(&manifest)
            })
            .await;
            match save_result {
                Ok(Ok(())) => {}
                Ok(Err(err)) => manifest_save_error = Some(err.to_string()),
                Err(err) => manifest_save_error = Some(err.to_string()),
            }
        }

        let summary = if let Some(err) = tree_error {
            format!(
                "已还原 {restored_files} 个文件，但对话树回退失败；检查点记录保留: {err}"
            )
        } else if let Some(err) = manifest_save_error {
            format!(
                "已还原 {restored_files} 个文件，但检查点清单保存失败，后续仍可重试: {err}"
            )
        } else if failed_files > 0 {
            format!(
                "已还原 {restored_files} 个文件，{failed_files} 个文件还原失败；失败检查点已保留，对话树未回退"
            )
        } else {
            format!("已回滚对话并还原 {restored_files} 个文件改动")
        };
        emit_rollback_result(&event_tx, &conversation_id, restored_files, summary).await;
    });
}

fn find_remote_session<'a>(
    sessions: &'a [(String, SshRemote, Option<SftpClient>)],
    session_catalog: &transom_core::SessionCatalog,
    target: &str,
) -> Option<(&'a SshRemote, &'a Option<SftpClient>)> {
    sessions
        .iter()
        .find(|(connection_id, _, _)| {
            connection_id == target
                || session_catalog.sessions.iter().any(|session| {
                    session.id.to_string() == *connection_id && session.name == target
                })
        })
        .map(|(_, remote, sftp)| (remote, sftp))
}

async fn rollback_conversation_leaf(
    store: Arc<AiStore>,
    conversation_id: &str,
    entry_id: &str,
) -> Result<(), String> {
    let store_tree = store.clone();
    let conversation = conversation_id.to_owned();
    let target_entry = entry_id.to_owned();
    tokio::task::spawn_blocking(move || {
        let tree = store_tree.load_session_tree(&conversation)?;
        let target_parent = tree
            .all_entries()
            .iter()
            .find(|entry| entry.id == target_entry)
            .and_then(|entry| entry.parent_id.clone());
        let mut index = store_tree.load_index()?;
        let Some(item) = index.iter_mut().find(|item| item.id == conversation) else {
            return Err(transom_ai::AiError::StorageFailed {
                target: conversation.clone(),
                reason: "对话索引不存在，无法回退活动叶节点".to_owned(),
            });
        };
        item.active_leaf_id = target_parent;
        item.updated_at = chrono::Utc::now().to_rfc3339();
        let updated = item.clone();
        drop(index);
        store_tree.upsert_conversation(updated)?;
        Ok::<(), transom_ai::AiError>(())
    })
    .await
    .map_err(|err| err.to_string())?
    .map_err(|err| err.to_string())
}

async fn emit_rollback_result(
    event_tx: &flume::Sender<AppEvent>,
    conversation_id: &str,
    restored_files: usize,
    summary: String,
) {
    if let Err(err) = event_tx
        .send_async(AppEvent::Ai(Box::new(AiAppEvent::ConversationRolledBack {
            conversation_id: conversation_id.to_owned(),
            restored_files,
            summary,
        })))
        .await
    {
        log::warn!("发送 AI 回滚结果事件失败: {err}");
    }
}

fn requeue_failed_records(
    manifest: &mut transom_ai::CheckpointManifest,
    rollback_turns: &[TurnCheckpoint],
    failed_records: &HashSet<(usize, usize)>,
) {
    for (turn_idx, turn) in rollback_turns.iter().enumerate() {
        for (record_idx, record) in turn.backups.iter().enumerate() {
            if failed_records.contains(&(turn_idx, record_idx)) {
                manifest.record_backup(&turn.entry_id, record.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::requeue_failed_records;
    use std::collections::HashSet;
    use transom_ai::{CheckpointManifest, FileBackupRecord, TurnCheckpoint};

    #[test]
    fn failed_rollback_records_are_retained_in_original_order() {
        let record = |path: &str| FileBackupRecord {
            target: "local".to_owned(),
            path: path.to_owned(),
            backup_path: Some(format!("{path}.bak")),
            existed_before: true,
        };
        let turns = vec![
            TurnCheckpoint {
                entry_id: "turn-1".to_owned(),
                backups: vec![record("/tmp/a"), record("/tmp/b")],
            },
            TurnCheckpoint {
                entry_id: "turn-2".to_owned(),
                backups: vec![record("/tmp/c")],
            },
        ];
        let mut manifest = CheckpointManifest {
            conversation_id: "conv-1".to_owned(),
            turns: Vec::new(),
        };
        let failed = HashSet::from([(0usize, 1usize), (1usize, 0usize)]);
        requeue_failed_records(&mut manifest, &turns, &failed);
        assert_eq!(manifest.turns.len(), 2);
        assert_eq!(manifest.turns[0].backups[0].path, "/tmp/b");
        assert_eq!(manifest.turns[1].backups[0].path, "/tmp/c");
    }
}
