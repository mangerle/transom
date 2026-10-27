//! SFTP 单项基础操作执行（下载、上传、目录递归、属性、文本编辑与目录对比）。

use super::progress::throttled_progress;
use crate::controller::sftp_finish::{
    PreviewRequest, finish_sftp, finish_sftp_none, finish_sftp_summary, read_preview,
    verify_transfer_result,
};
use transom_core::{
    AppError, AppEvent, ConnectionId, ErrorCode, SftpAppEvent, SftpOperation, TabId, TransferId,
};
use transom_net::SftpClient;

/// 编辑器读取远端文本的最长等待时间。
///
/// 与预览不同，编辑是用户主动发起且界面正在等的操作，超时过短会把慢链路上一次
/// 正常读取误判为失败。按内置编辑器的 8 MiB 上限与约 1 Mbps 的慢链路估算需要
/// 60 秒以上，留一倍余量取 2 分钟：仍能兜住"对端挂起导致协程永久泄漏"，
/// 又不至于让用户对着读取中无限等待。
const TEXT_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// 编辑器回传文本的最长等待时间。写入需要远端落盘并回读确认，比读取更慢，取 5 分钟。
const TEXT_WRITE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

pub(crate) async fn execute_sftp_op(
    event_tx: flume::Sender<AppEvent>,
    tab_id: TabId,
    connection_id: ConnectionId,
    op_id: TransferId,
    client: SftpClient,
    op: SftpOperation,
) {
    match op {
        SftpOperation::List { path } => match client.list(&path).await {
            Ok((path, entries)) => {
                let _ = event_tx
                    .send_async(
                        SftpAppEvent::SftpListing {
                            tab_id,
                            connection_id,
                            path,
                            entries,
                        }
                        .into(),
                    )
                    .await;
            }
            Err(error) => {
                let _ = event_tx
                    .send_async(
                        SftpAppEvent::SftpFailed {
                            tab_id,
                            connection_id,
                            op_id,
                            error,
                        }
                        .into(),
                    )
                    .await;
            }
        },
        SftpOperation::Download {
            remote_path,
            local_path,
            resume,
            verify,
        } => {
            let result = client
                .download_resumable(
                    &remote_path,
                    &local_path,
                    resume,
                    throttled_progress(event_tx.clone(), tab_id, connection_id, op_id),
                )
                .await;
            let result =
                verify_transfer_result(&client, result, verify, &local_path, &remote_path).await;
            finish_sftp(event_tx, tab_id, connection_id, op_id, result).await;
        }
        SftpOperation::Upload {
            local_path,
            remote_path,
            resume,
            verify,
        } => {
            let result = client
                .upload_resumable(
                    &local_path,
                    &remote_path,
                    resume,
                    throttled_progress(event_tx.clone(), tab_id, connection_id, op_id),
                )
                .await;
            let result =
                verify_transfer_result(&client, result, verify, &local_path, &remote_path).await;
            finish_sftp(event_tx, tab_id, connection_id, op_id, result).await;
        }
        SftpOperation::UploadDir {
            local_path,
            remote_path,
            resume,
            filter,
            verify,
        } => {
            let result = client
                .upload_dir(
                    &local_path,
                    &remote_path,
                    resume,
                    &filter,
                    verify,
                    throttled_progress(event_tx.clone(), tab_id, connection_id, op_id),
                )
                .await;
            finish_sftp_summary(event_tx, tab_id, connection_id, op_id, result).await;
        }
        SftpOperation::DownloadDir {
            remote_path,
            local_path,
            resume,
            filter,
            verify,
        } => {
            let result = client
                .download_dir(
                    &remote_path,
                    &local_path,
                    resume,
                    &filter,
                    verify,
                    throttled_progress(event_tx.clone(), tab_id, connection_id, op_id),
                )
                .await;
            finish_sftp_summary(event_tx, tab_id, connection_id, op_id, result).await;
        }
        SftpOperation::Mkdir { path } => {
            let result = client.create_dir(&path).await;
            finish_sftp_none(event_tx, tab_id, connection_id, op_id, result).await;
        }
        SftpOperation::Rename { from, to } => {
            let result = client.rename(&from, &to).await;
            finish_sftp_none(event_tx, tab_id, connection_id, op_id, result).await;
        }
        SftpOperation::Delete { path, is_dir } => {
            let result = client.delete(&path, is_dir).await;
            finish_sftp_none(event_tx, tab_id, connection_id, op_id, result).await;
        }
        SftpOperation::DeleteMany { targets } => {
            let mut result = Ok(());
            for target in targets {
                if let Err(error) = client.delete(&target.path, target.is_dir).await {
                    result = Err(error);
                    break;
                }
            }
            finish_sftp_none(event_tx, tab_id, connection_id, op_id, result).await;
        }
        SftpOperation::Chmod { path, permissions } => {
            let result = client.chmod(&path, permissions).await;
            finish_sftp_none(event_tx, tab_id, connection_id, op_id, result).await;
        }
        SftpOperation::Stat { path } => match client.stat(&path).await {
            Ok(metadata) => {
                let _ = event_tx
                    .send_async(
                        SftpAppEvent::SftpMetadata {
                            tab_id,
                            connection_id,
                            op_id,
                            metadata,
                        }
                        .into(),
                    )
                    .await;
            }
            Err(error) => {
                let _ = event_tx
                    .send_async(
                        SftpAppEvent::SftpFailed {
                            tab_id,
                            connection_id,
                            op_id,
                            error,
                        }
                        .into(),
                    )
                    .await;
            }
        },
        SftpOperation::Preview {
            path,
            mode,
            max_bytes,
        } => {
            read_preview(PreviewRequest {
                client: &client,
                event_tx,
                tab_id,
                connection_id,
                op_id,
                path,
                mode,
                max_bytes,
            })
            .await;
        }
        SftpOperation::ReadText { path, max_bytes } => {
            let result =
                tokio::time::timeout(TEXT_READ_TIMEOUT, client.read_text(&path, max_bytes)).await;
            match result {
                Ok(Ok((content, meta))) => {
                    let _ = event_tx
                        .send_async(
                            SftpAppEvent::SftpTextRead {
                                tab_id,
                                connection_id,
                                op_id,
                                content,
                                meta,
                            }
                            .into(),
                        )
                        .await;
                }
                Ok(Err(error)) => {
                    let _ = event_tx
                        .send_async(
                            SftpAppEvent::SftpTextFailed {
                                tab_id,
                                connection_id,
                                op_id,
                                error,
                            }
                            .into(),
                        )
                        .await;
                }
                Err(_) => {
                    let _ = event_tx
                        .send_async(
                            SftpAppEvent::SftpTextFailed {
                                tab_id,
                                connection_id,
                                op_id,
                                error: AppError::with_detail(
                                    ErrorCode::SftpFailed,
                                    format!(
                                        "读取远端文本超时（{} 秒）",
                                        TEXT_READ_TIMEOUT.as_secs()
                                    ),
                                ),
                            }
                            .into(),
                        )
                        .await;
                }
            }
        }
        SftpOperation::WriteText {
            path,
            content,
            expected,
            force,
            backup,
        } => {
            let result = tokio::time::timeout(
                TEXT_WRITE_TIMEOUT,
                client.write_text(&path, &content, expected, force, backup),
            )
            .await;
            match result {
                Ok(Ok(meta)) => {
                    let _ = event_tx
                        .send_async(
                            SftpAppEvent::SftpTextWritten {
                                tab_id,
                                connection_id,
                                op_id,
                                meta,
                            }
                            .into(),
                        )
                        .await;
                }
                Ok(Err(error)) if error.code == ErrorCode::SftpTextConflict => {
                    let _ = event_tx
                        .send_async(
                            SftpAppEvent::SftpTextConflict {
                                tab_id,
                                connection_id,
                                op_id,
                                error,
                            }
                            .into(),
                        )
                        .await;
                }
                Ok(Err(error)) => {
                    let _ = event_tx
                        .send_async(
                            SftpAppEvent::SftpTextFailed {
                                tab_id,
                                connection_id,
                                op_id,
                                error,
                            }
                            .into(),
                        )
                        .await;
                }
                Err(_) => {
                    let _ = event_tx
                        .send_async(
                            SftpAppEvent::SftpTextFailed {
                                tab_id,
                                connection_id,
                                op_id,
                                error: AppError::with_detail(
                                    ErrorCode::SftpFailed,
                                    format!(
                                        "回传远端文本超时（{} 秒）",
                                        TEXT_WRITE_TIMEOUT.as_secs()
                                    ),
                                ),
                            }
                            .into(),
                        )
                        .await;
                }
            }
        }
        SftpOperation::Symlink { target, link_path } => {
            let result = client.symlink(&target, &link_path).await;
            finish_sftp_none(event_tx, tab_id, connection_id, op_id, result).await;
        }
        SftpOperation::CompareDirs {
            local_path,
            remote_path,
            filter,
        } => match client
            .compare_directories(&local_path, &remote_path, &filter)
            .await
        {
            Ok(comparison) => {
                let _ = event_tx
                    .send_async(
                        SftpAppEvent::SftpDirCompared {
                            tab_id,
                            connection_id,
                            op_id,
                            comparison,
                        }
                        .into(),
                    )
                    .await;
            }
            Err(error) => {
                let _ = event_tx
                    .send_async(
                        SftpAppEvent::SftpFailed {
                            tab_id,
                            connection_id,
                            op_id,
                            error,
                        }
                        .into(),
                    )
                    .await;
            }
        },
        SftpOperation::SyncDirs {
            local_path,
            remote_path,
            items,
            verify,
        } => {
            let result = client
                .sync_items(
                    &local_path,
                    &remote_path,
                    &items,
                    verify,
                    throttled_progress(event_tx.clone(), tab_id, connection_id, op_id),
                )
                .await;
            finish_sftp_summary(event_tx, tab_id, connection_id, op_id, result).await;
        }
        SftpOperation::RemoteTransfer { .. } => {}
    }
}
