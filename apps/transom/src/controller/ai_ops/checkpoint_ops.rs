//! 按对话轮次的 Checkpoint 备份记录与一键回滚执行器。

use std::path::{Path, PathBuf};
use std::time::Duration;

use transom_ai::{CheckpointManifest, FileBackupRecord, RemoteFsExecutor};
use transom_core::{AiAppEvent, AiToolCardDto, AppEvent};
use transom_net::{SftpClient, SshRemote};
use transom_store::AiStore;
use uuid::Uuid;

use super::app_bridge::AppBridgeDeps;
use super::bridge::SshRemoteFsBridge;
use crate::controller::state::{ControllerCtx, ControllerState};

pub(super) fn store_ai_root(store: &AiStore) -> PathBuf {
    store
        .custom_skills_dir()
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| store.custom_skills_dir())
}

/// 在本机或远端写入文件前保存覆盖前快照到检查点清单。
pub(super) async fn record_local_backup(deps: &AppBridgeDeps, path: &str) {
    let Some(conv_id) = deps.conversation_id.as_deref() else {
        return;
    };
    let Some(entry_id) = deps.current_turn_entry_id.read().clone() else {
        return;
    };
    let root = store_ai_root(&deps.ai_store);
    let src = Path::new(path);
    let (existed_before, backup_path) = if src.exists() {
        let blob_dir = root.join("checkpoints").join("blobs");
        let _ = tokio::fs::create_dir_all(&blob_dir).await;
        let blob = blob_dir.join(format!("{}.bak", Uuid::new_v4()));
        match tokio::fs::copy(src, &blob).await {
            Ok(_) => (true, Some(blob.display().to_string())),
            Err(err) => {
                log::warn!("复制本机检查点备份失败（{path}）: {err}");
                (true, None)
            }
        }
    } else {
        (false, None)
    };

    append_record(
        &root,
        conv_id,
        &entry_id,
        FileBackupRecord {
            target: "local".to_owned(),
            path: path.to_owned(),
            backup_path,
            existed_before,
        },
    );
}

/// 在远端写入文件前读取原文件内容并保存快照到本机检查点目录。
pub(super) async fn record_remote_backup(
    deps: &AppBridgeDeps,
    target_label: &str,
    path: &str,
    fs: &SshRemoteFsBridge,
) {
    let Some(conv_id) = deps.conversation_id.as_deref() else {
        return;
    };
    let Some(entry_id) = deps.current_turn_entry_id.read().clone() else {
        return;
    };
    let root = store_ai_root(&deps.ai_store);
    let (existed_before, backup_path) = match fs.read_file(path, 2 * 1024 * 1024).await {
        Ok(content) => {
            let blob_dir = root.join("checkpoints").join("blobs");
            let _ = tokio::fs::create_dir_all(&blob_dir).await;
            let blob = blob_dir.join(format!("{}.bak", Uuid::new_v4()));
            match tokio::fs::write(&blob, content.as_bytes()).await {
                Ok(_) => (true, Some(blob.display().to_string())),
                Err(err) => {
                    log::warn!("保存远端检查点快照失败（{path}）: {err}");
                    (true, None)
                }
            }
        }
        Err(_) => (false, None),
    };

    append_record(
        &root,
        conv_id,
        &entry_id,
        FileBackupRecord {
            target: target_label.to_owned(),
            path: path.to_owned(),
            backup_path,
            existed_before,
        },
    );
}

fn append_record(root: &Path, conv_id: &str, entry_id: &str, record: FileBackupRecord) {
    if let Ok(mut manifest) = CheckpointManifest::load(root, conv_id) {
        manifest.record_backup(entry_id, record);
        if let Err(err) = manifest.save(root) {
            log::warn!("写入检查点清单失败: {err}");
        }
    }
}

/// 执行按轮次回滚：还原文件快照并将对话树的活动叶节点回退到指定轮次之前。
pub(super) fn spawn_rollback_checkpoint(
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

    state.ai_set.spawn(async move {
        let root = store_ai_root(&store);
        let mut restored_files = 0usize;

        if let Ok(mut manifest) = CheckpointManifest::load(&root, &conversation_id) {
            let records = manifest.drain_rollback_records(&entry_id);
            for rec in records {
                if rec.target == "local" {
                    if restore_local(&rec).await {
                        restored_files += 1;
                    }
                } else {
                    let matched_remote = sessions.iter().find(|(cid, _, _)| {
                        cid == &rec.target
                            || session_catalog
                                .sessions
                                .iter()
                                .any(|s| s.id.to_string() == *cid && s.name == rec.target)
                    });
                    if let Some((_, remote, sftp)) = matched_remote
                        && restore_remote(&rec, remote, sftp.clone()).await
                    {
                        restored_files += 1;
                    }
                }
            }
            let _ = manifest.save(&root);
        }

        // 回退对话树活动叶节点到目标轮次的父节点
        if let Ok(tree) = store.load_session_tree(&conversation_id) {
            let target_parent = tree
                .all_entries()
                .iter()
                .find(|e| e.id == entry_id)
                .and_then(|e| e.parent_id.clone());
            if let Ok(mut index) = store.load_index()
                && let Some(item) = index.iter_mut().find(|i| i.id == conversation_id)
            {
                item.active_leaf_id = target_parent;
                item.updated_at = chrono::Utc::now().to_rfc3339();
                let _ = store.upsert_conversation(item.clone());
            }
        }

        let summary = format!("已回滚对话并还原 {restored_files} 个文件改动");
        let _ = event_tx
            .send_async(AppEvent::Ai(Box::new(AiAppEvent::ConversationRolledBack {
                conversation_id,
                restored_files,
                summary,
            })))
            .await;
    });
}

async fn restore_local(rec: &FileBackupRecord) -> bool {
    let dst = Path::new(&rec.path);
    if rec.existed_before {
        if let Some(bak) = &rec.backup_path
            && Path::new(bak).exists()
        {
            return tokio::fs::copy(bak, dst).await.is_ok();
        }
        false
    } else if dst.exists() {
        tokio::fs::remove_file(dst).await.is_ok()
    } else {
        false
    }
}

async fn restore_remote(
    rec: &FileBackupRecord,
    remote: &SshRemote,
    sftp: Option<SftpClient>,
) -> bool {
    let fs = SshRemoteFsBridge {
        remote: remote.clone(),
        sftp,
    };
    if rec.existed_before {
        if let Some(bak) = &rec.backup_path
            && let Ok(content) = tokio::fs::read_to_string(bak).await
        {
            return fs.write_file(&rec.path, &content, false).await.is_ok();
        }
        false
    } else {
        let _ = tokio::time::timeout(Duration::from_secs(10), async {
            let _ = fs.delete(&rec.path, false).await;
        })
        .await;
        true
    }
}

/// 将指定远端文件一键还原至修改前在本机保存的快照内容。
pub(super) fn spawn_revert_remote_file(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    host_label: String,
    target_path: String,
    backup_path: String,
    conversation_id: Option<String>,
) {
    let session_catalog = ctx.session_store.load().unwrap_or_default();
    let sessions: Vec<(String, SshRemote, Option<SftpClient>)> = state
        .sessions
        .values()
        .filter_map(|m| {
            m.remote
                .clone()
                .map(|r| (m.connection_id.to_string(), r, m.sftp_client.clone()))
        })
        .collect();
    let target_session = sessions
        .iter()
        .find(|(cid, _, _)| {
            !host_label.is_empty()
                && (cid == &host_label
                    || session_catalog
                        .sessions
                        .iter()
                        .any(|s| s.id.to_string() == *cid && s.name == host_label))
        })
        .or_else(|| (sessions.len() == 1).then(|| &sessions[0]))
        .map(|(_, r, s)| (r.clone(), s.clone()));

    let allowed_root = store_ai_root(&ctx.ai_store)
        .join("checkpoints")
        .join("blobs");
    let event_tx = ctx.event_tx.clone();
    let conv_id = conversation_id.unwrap_or_default();

    state.ai_set.spawn(async move {
        let (ok, output) = match validate_and_read_backup(&allowed_root, &backup_path).await {
            Ok(content) => match target_session {
                Some((remote, sftp)) => {
                    let fs = SshRemoteFsBridge { remote, sftp };
                    match tokio::time::timeout(
                        Duration::from_secs(15),
                        fs.write_file(&target_path, &content, false),
                    )
                    .await
                    {
                        Ok(Ok(())) => (
                            true,
                            format!("已成功将远端文件还原至修改前快照: {target_path}"),
                        ),
                        Ok(Err(err)) => {
                            (false, format!("还原远端文件失败（{target_path}）: {err}"))
                        }
                        Err(_) => (false, format!("还原远端文件超时（{target_path}）")),
                    }
                }
                None => (
                    false,
                    format!("未找到主机「{host_label}」的活跃 SSH 会话，无法执行远端文件还原"),
                ),
            },
            Err(reason) => (false, reason),
        };

        let dto = AiToolCardDto {
            conversation_id: conv_id,
            call_id: String::new(),
            running: false,
            tool_name: "remote_fs_revert".to_owned(),
            host_label,
            risk_label: "还原".to_owned(),
            summary: format!("还原 {target_path}"),
            output,
            ok,
            duration_ms: 0,
            diff_preview: None,
            revert_target_path: None,
            revert_backup_path: None,
        };
        let _ = event_tx
            .send_async(AppEvent::Ai(Box::new(AiAppEvent::ToolCard(dto))))
            .await;
    });
}

async fn validate_and_read_backup(
    allowed_root: &Path,
    backup_path: &str,
) -> Result<String, String> {
    let canon_root = tokio::fs::canonicalize(allowed_root)
        .await
        .map_err(|_| "快照目录不存在，无法还原".to_owned())?;
    let canon_file = tokio::fs::canonicalize(backup_path)
        .await
        .map_err(|err| format!("找不到备份快照文件（{backup_path}）: {err}"))?;
    if !canon_file.starts_with(&canon_root) {
        return Err("拒绝访问受控快照目录之外的文件路径".to_owned());
    }
    tokio::fs::read_to_string(&canon_file)
        .await
        .map_err(|err| format!("读取本地快照文件失败: {err}"))
}
