//! SFTP 单项基础操作执行（下载、上传、目录递归、属性、文本编辑与目录对比）。
//!
//! 设计原理（Why）：
//! - 将平铺的 6 个参数收敛为 [`SftpExecutionContext`] 上下文对象；
//! - 采用 Handler 模式将各操作分支独立为细粒度处理函数，主分发函数控制在 35 行以内；
//! - 各处理函数单一职责，便于独立测试、维护与添加超时保护。

mod fs;
mod query;
mod sync;
mod text;
mod transfer;

use self::fs::{
    handle_chmod, handle_delete, handle_delete_many, handle_mkdir, handle_rename, handle_symlink,
};
use self::query::{handle_list, handle_stat};
use self::sync::{handle_compare_dirs, handle_sync_dirs};
use self::text::{handle_preview, handle_read_text, handle_write_text};
use self::transfer::{handle_download, handle_download_dir, handle_upload, handle_upload_dir};
pub(super) use super::progress::throttled_progress;
use transom_core::{AppEvent, ConnectionId, SftpOperation, TabId, TransferId};
use transom_net::SftpClient;

/// SFTP 任务执行上下文对象：收敛原平铺参数。
pub(crate) struct SftpExecutionContext {
    pub(crate) event_tx: flume::Sender<AppEvent>,
    pub(crate) tab_id: TabId,
    pub(crate) connection_id: ConnectionId,
    pub(crate) op_id: TransferId,
    pub(crate) client: SftpClient,
}

/// 执行单项 SFTP 操作的主入口分发。
pub(crate) async fn execute_sftp_op(ctx: SftpExecutionContext, op: SftpOperation) {
    match op {
        SftpOperation::List { path } => handle_list(&ctx, path).await,
        SftpOperation::Download {
            remote_path,
            local_path,
            resume,
            verify,
        } => handle_download(&ctx, remote_path, local_path, resume, verify).await,
        SftpOperation::Upload {
            local_path,
            remote_path,
            resume,
            verify,
        } => handle_upload(&ctx, local_path, remote_path, resume, verify).await,
        SftpOperation::UploadDir {
            local_path,
            remote_path,
            resume,
            filter,
            verify,
        } => handle_upload_dir(&ctx, local_path, remote_path, resume, filter, verify).await,
        SftpOperation::DownloadDir {
            remote_path,
            local_path,
            resume,
            filter,
            verify,
        } => handle_download_dir(&ctx, remote_path, local_path, resume, filter, verify).await,
        SftpOperation::Mkdir { path } => handle_mkdir(&ctx, path).await,
        SftpOperation::Rename { from, to } => handle_rename(&ctx, from, to).await,
        SftpOperation::Delete { path, is_dir } => handle_delete(&ctx, path, is_dir).await,
        SftpOperation::DeleteMany { targets } => handle_delete_many(&ctx, targets).await,
        SftpOperation::Chmod { path, permissions } => handle_chmod(&ctx, path, permissions).await,
        SftpOperation::Stat { path } => handle_stat(&ctx, path).await,
        SftpOperation::Preview {
            path,
            mode,
            max_bytes,
        } => handle_preview(&ctx, path, mode, max_bytes).await,
        SftpOperation::ReadText { path, max_bytes } => {
            handle_read_text(&ctx, path, max_bytes).await
        }
        SftpOperation::WriteText {
            path,
            content,
            expected,
            force,
            backup,
        } => handle_write_text(&ctx, path, content, expected, force, backup).await,
        SftpOperation::Symlink { target, link_path } => {
            handle_symlink(&ctx, target, link_path).await
        }
        SftpOperation::CompareDirs {
            local_path,
            remote_path,
            filter,
        } => handle_compare_dirs(&ctx, local_path, remote_path, filter).await,
        SftpOperation::SyncDirs {
            local_path,
            remote_path,
            items,
            verify,
        } => handle_sync_dirs(&ctx, local_path, remote_path, items, verify).await,
        SftpOperation::RemoteTransfer { .. } => {}
    }
}
