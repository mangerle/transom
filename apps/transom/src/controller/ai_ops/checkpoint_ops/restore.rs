//! 本机与远端文件快照还原。

use std::path::Path;
use tokio_util::sync::CancellationToken;
use transom_ai::{FileBackupRecord, RemoteFsExecutor, ToolIoContext};
use transom_core::{AiAppEvent, AiToolCardDto, AppEvent};
use transom_net::{SftpClient, SshRemote};

use super::super::bridge::SshRemoteFsBridge;
use super::{MAX_CHECKPOINT_BACKUP_BYTES, REMOTE_RESTORE_TIMEOUT, store_ai_root};
use crate::controller::state::{ControllerCtx, ControllerState};

pub(super) async fn restore_local(record: &FileBackupRecord) -> Result<(), String> {
    let destination = Path::new(&record.path);
    if record.existed_before {
        let backup = record
            .backup_path
            .as_deref()
            .ok_or_else(|| format!("缺少本机快照路径: {}", record.path))?;
        tokio::fs::copy(backup, destination)
            .await
            .map(|_| ())
            .map_err(|err| format!("还原本机文件失败（{}）: {err}", record.path))
    } else {
        match tokio::fs::metadata(destination).await {
            Ok(_) => tokio::fs::remove_file(destination)
                .await
                .map_err(|err| format!("删除本机新增文件失败（{}）: {err}", record.path)),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(format!("探测本机文件失败（{}）: {err}", record.path)),
        }
    }
}

pub(super) async fn restore_remote(
    record: &FileBackupRecord,
    remote: &SshRemote,
    sftp: Option<SftpClient>,
    io: &ToolIoContext,
) -> Result<(), String> {
    let fs = SshRemoteFsBridge {
        remote: remote.clone(),
        sftp,
    };
    if record.existed_before {
        let backup = record
            .backup_path
            .as_deref()
            .ok_or_else(|| format!("缺少远端快照路径: {}", record.path))?;
        let content = read_backup_file(Path::new(backup)).await?;
        fs.write_file_with_io(&record.path, &content, false, io)
            .await
            .map_err(|err| format!("还原远端文件失败（{}）: {err}", record.path))
    } else if fs
        .file_exists_with_io(&record.path, io)
        .await
        .map_err(|err| format!("探测远端文件失败（{}）: {err}", record.path))?
    {
        fs.delete_with_io(&record.path, false, io)
            .await
            .map_err(|err| format!("删除远端新增文件失败（{}）: {err}", record.path))
    } else {
        Ok(())
    }
}

async fn read_backup_file(path: &Path) -> Result<String, String> {
    let metadata = tokio::fs::metadata(path)
        .await
        .map_err(|err| format!("读取检查点快照属性失败（{}）: {err}", path.display()))?;
    if metadata.len() > MAX_CHECKPOINT_BACKUP_BYTES as u64 {
        return Err(format!(
            "检查点快照超过体积上限（{}，上限 {} 字节）",
            path.display(),
            MAX_CHECKPOINT_BACKUP_BYTES
        ));
    }
    tokio::fs::read_to_string(path)
        .await
        .map_err(|err| format!("读取检查点快照失败（{}）: {err}", path.display()))
}

/// 将指定远端文件一键还原至修改前在本机保存的快照内容。
pub(in crate::controller::ai_ops) fn spawn_revert_remote_file(
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
        .map(|(_, remote, sftp)| (remote.clone(), sftp.clone()));

    let allowed_root = store_ai_root(&ctx.ai_store)
        .join("checkpoints")
        .join("blobs");
    let event_tx = ctx.event_tx.clone();
    let conv_id = conversation_id.unwrap_or_default();

    state.ai.run_set.spawn(async move {
        let io = ToolIoContext::new(REMOTE_RESTORE_TIMEOUT, CancellationToken::new());
        let (ok, output) = match validate_and_read_backup(&allowed_root, &backup_path).await {
            Ok(content) => match target_session {
                Some((remote, sftp)) => {
                    let fs = SshRemoteFsBridge { remote, sftp };
                    match fs
                        .write_file_with_io(&target_path, &content, false, &io)
                        .await
                    {
                        Ok(()) => (
                            true,
                            format!("已成功将远端文件还原至修改前快照: {target_path}"),
                        ),
                        Err(err) => (false, format!("还原远端文件失败（{target_path}）: {err}")),
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
            run_id: String::new(),
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
        if let Err(err) = event_tx
            .send_async(AppEvent::Ai(Box::new(AiAppEvent::ToolCard(dto))))
            .await
        {
            log::warn!("发送远端文件还原结果失败: {err}");
        }
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
        return Err("拒绝访问受控快照目录之外的路径".to_owned());
    }
    read_backup_file(&canon_file).await
}

#[cfg(test)]
mod tests {
    use super::{MAX_CHECKPOINT_BACKUP_BYTES, restore_local};
    use transom_ai::FileBackupRecord;

    #[tokio::test]
    async fn restore_local_deletes_file_created_after_checkpoint() {
        let dir = tempfile::tempdir().expect("创建临时目录失败");
        let target = dir.path().join("created.txt");
        tokio::fs::write(&target, "new")
            .await
            .expect("创建测试文件失败");
        let record = FileBackupRecord {
            target: "local".to_owned(),
            path: target.display().to_string(),
            backup_path: None,
            existed_before: false,
        };
        restore_local(&record).await.expect("回滚新增文件失败");
        assert!(!target.exists());
    }

    #[tokio::test]
    async fn restore_local_never_treats_missing_backup_as_success() {
        let dir = tempfile::tempdir().expect("创建临时目录失败");
        let target = dir.path().join("existing.txt");
        tokio::fs::write(&target, "current")
            .await
            .expect("创建测试文件失败");
        let record = FileBackupRecord {
            target: "local".to_owned(),
            path: target.display().to_string(),
            backup_path: None,
            existed_before: true,
        };
        assert!(restore_local(&record).await.is_err());
        assert!(target.exists());
    }

    #[test]
    fn checkpoint_backup_limit_is_explicit() {
        assert_eq!(MAX_CHECKPOINT_BACKUP_BYTES, 2 * 1024 * 1024);
    }
}
