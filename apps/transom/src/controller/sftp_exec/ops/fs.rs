//! SFTP 文件系统类操作：目录创建、重命名、删除、权限与软链接。

use super::SftpExecutionContext;
use crate::controller::sftp_finish::finish_sftp_none;
use transom_core::RemoteDeleteTarget;
pub(super) async fn handle_mkdir(ctx: &SftpExecutionContext, path: String) {
    let result = ctx.client.create_dir(&path).await;
    finish_sftp_none(
        ctx.event_tx.clone(),
        ctx.tab_id,
        ctx.connection_id,
        ctx.op_id,
        result,
    )
    .await;
}

pub(super) async fn handle_rename(ctx: &SftpExecutionContext, from: String, to: String) {
    let result = ctx.client.rename(&from, &to).await;
    finish_sftp_none(
        ctx.event_tx.clone(),
        ctx.tab_id,
        ctx.connection_id,
        ctx.op_id,
        result,
    )
    .await;
}

pub(super) async fn handle_delete(ctx: &SftpExecutionContext, path: String, is_dir: bool) {
    let result = ctx.client.delete(&path, is_dir).await;
    finish_sftp_none(
        ctx.event_tx.clone(),
        ctx.tab_id,
        ctx.connection_id,
        ctx.op_id,
        result,
    )
    .await;
}

pub(super) async fn handle_delete_many(
    ctx: &SftpExecutionContext,
    targets: Vec<RemoteDeleteTarget>,
) {
    let mut result = Ok(());
    for target in targets {
        if let Err(error) = ctx.client.delete(&target.path, target.is_dir).await {
            result = Err(error);
            break;
        }
    }
    finish_sftp_none(
        ctx.event_tx.clone(),
        ctx.tab_id,
        ctx.connection_id,
        ctx.op_id,
        result,
    )
    .await;
}

pub(super) async fn handle_chmod(ctx: &SftpExecutionContext, path: String, permissions: u32) {
    let result = ctx.client.chmod(&path, permissions).await;
    finish_sftp_none(
        ctx.event_tx.clone(),
        ctx.tab_id,
        ctx.connection_id,
        ctx.op_id,
        result,
    )
    .await;
}

pub(super) async fn handle_symlink(ctx: &SftpExecutionContext, target: String, link_path: String) {
    let result = ctx.client.symlink(&target, &link_path).await;
    finish_sftp_none(
        ctx.event_tx.clone(),
        ctx.tab_id,
        ctx.connection_id,
        ctx.op_id,
        result,
    )
    .await;
}
