//! 文件写前备份与检查点记录追加。

use std::path::Path;

use transom_ai::{FileBackupRecord, RemoteFsExecutor, ToolError};
use uuid::Uuid;

use super::super::app_bridge::AppBridgeDeps;
use super::super::bridge::SshRemoteFsBridge;
use super::{
    MAX_CHECKPOINT_BACKUP_BYTES, checkpoint_blob_dir, checkpoint_context, checkpoint_error,
};

/// 在写入本机文件前保存覆盖前快照。
///
/// # Errors
/// 无法区分路径是否存在、备份复制失败或清单落盘失败时返回 [`ToolError`]；
/// 调用方必须中止后续写操作。
pub(in crate::controller::ai_ops) async fn record_local_backup(
    deps: &AppBridgeDeps,
    path: &str,
) -> Result<(), ToolError> {
    let Some((conversation_id, entry_id)) = checkpoint_context(deps) else {
        return Ok(());
    };
    let src = Path::new(path);
    let existed_before = match tokio::fs::metadata(src).await {
        Ok(_) => true,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => false,
        Err(err) => {
            return Err(checkpoint_error(format!(
                "探测本机路径失败（{path}）: {err}"
            )));
        }
    };

    let backup_path = if existed_before {
        let blob_dir = checkpoint_blob_dir(deps);
        tokio::fs::create_dir_all(&blob_dir)
            .await
            .map_err(|err| checkpoint_error(format!("创建本机备份目录失败: {err}")))?;
        let blob = blob_dir.join(format!("{}.bak", Uuid::new_v4()));
        tokio::fs::copy(src, &blob)
            .await
            .map_err(|err| checkpoint_error(format!("复制本机检查点备份失败（{path}）: {err}")))?;
        Some(blob.display().to_string())
    } else {
        None
    };

    append_record(
        deps,
        &conversation_id,
        &entry_id,
        FileBackupRecord {
            target: "local".to_owned(),
            path: path.to_owned(),
            backup_path,
            existed_before,
        },
    )
}

/// 在写入远端文件前读取原文件内容并保存本机快照。
///
/// # Errors
/// 远端存在性探测、文件读取、快照写入或清单落盘失败时返回 [`ToolError`]；
/// 调用方必须中止后续远端写操作。
pub(in crate::controller::ai_ops) async fn record_remote_backup(
    deps: &AppBridgeDeps,
    target_label: &str,
    path: &str,
    fs: &SshRemoteFsBridge,
) -> Result<(), ToolError> {
    let Some((conversation_id, entry_id)) = checkpoint_context(deps) else {
        return Ok(());
    };
    let existed_before = fs.file_exists(path).await?;
    let backup_path = if existed_before {
        let content = fs.read_file(path, MAX_CHECKPOINT_BACKUP_BYTES).await?;
        let blob_dir = checkpoint_blob_dir(deps);
        tokio::fs::create_dir_all(&blob_dir)
            .await
            .map_err(|err| checkpoint_error(format!("创建远端备份目录失败: {err}")))?;
        let blob = blob_dir.join(format!("{}.bak", Uuid::new_v4()));
        tokio::fs::write(&blob, content.as_bytes())
            .await
            .map_err(|err| checkpoint_error(format!("保存远端检查点快照失败（{path}）: {err}")))?;
        Some(blob.display().to_string())
    } else {
        None
    };

    append_record(
        deps,
        &conversation_id,
        &entry_id,
        FileBackupRecord {
            target: target_label.to_owned(),
            path: path.to_owned(),
            backup_path,
            existed_before,
        },
    )
}

fn append_record(
    deps: &AppBridgeDeps,
    conversation_id: &str,
    entry_id: &str,
    record: FileBackupRecord,
) -> Result<(), ToolError> {
    deps.stores
        .ai_store
        .append_checkpoint_record(conversation_id, entry_id, record)
        .map_err(|err| checkpoint_error(format!("写入检查点清单失败: {err}")))
}
