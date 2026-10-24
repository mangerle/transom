//! SFTP 操作的调度入口、任务执行体与结果回传助手。

use super::sftp_finish::{
    PreviewRequest, finish_sftp, finish_sftp_none, finish_sftp_summary, read_preview,
    verify_transfer_result,
};
use std::collections::HashMap;
use transom_core::{AppError, AppEvent, ConnectionId, ErrorCode, SftpOperation, TabId, TransferId};
use transom_net::{SftpClient, SshRemote};

/// 传输进度事件的最小发送间隔。
///
/// 传输层每 64KB 回调一次，内网下每秒可达数千次；直接透传会让 UI 以相同
/// 频率整帧重绘。进度是单调可覆盖的状态量且终值由 `SftpFinished` 事件兜底，
/// 中间样本按 100ms 窗口抽样丢弃不损失最终一致性，人眼对进度条刷新的
/// 分辨率也远低于 10Hz。
const PROGRESS_EVENT_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);

/// 构造按 [`PROGRESS_EVENT_INTERVAL`] 节流的进度回调。
fn throttled_progress(
    progress_tx: flume::Sender<AppEvent>,
    tab_id: TabId,
    connection_id: ConnectionId,
    op_id: TransferId,
) -> impl FnMut(u64, Option<u64>) + Send {
    let mut last_sent: Option<std::time::Instant> = None;
    move |transferred, total| {
        let now = std::time::Instant::now();
        if last_sent.is_none_or(|at| now.duration_since(at) >= PROGRESS_EVENT_INTERVAL) {
            last_sent = Some(now);
            // 回调运行在非 async 上下文：队列满时快速失败丢弃本条样本。
            // 这是显式背压——UI 消费停滞时宁可进度刷新变慢，也不阻塞传输线程。
            let _ = progress_tx.try_send(AppEvent::SftpProgress {
                tab_id,
                connection_id,
                op_id,
                transferred,
                total,
            });
        }
    }
}

/// 编辑器读取远端文本的最长等待时间。
///
/// 与预览不同，编辑是用户主动发起且界面正在等的操作，超时过短会把慢链路上一次
/// 正常读取误判为失败。按内置编辑器的 8 MiB 上限与约 1 Mbps 的慢链路估算需要
/// 60 秒以上，留一倍余量取 2 分钟：仍能兜住"对端挂起导致协程永久泄漏"，
/// 又不至于让用户对着读取中无限等待。
const TEXT_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// 编辑器回传文本的最长等待时间。写入需要远端落盘并回读确认，比读取更慢，取 5 分钟。
const TEXT_WRITE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

pub(super) async fn execute_sftp_op(
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
                    .send_async(AppEvent::SftpListing {
                        tab_id,
                        connection_id,
                        path,
                        entries,
                    })
                    .await;
            }
            Err(error) => {
                let _ = event_tx
                    .send_async(AppEvent::SftpFailed {
                        tab_id,
                        connection_id,
                        op_id,
                        error,
                    })
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
                    .send_async(AppEvent::SftpMetadata {
                        tab_id,
                        connection_id,
                        op_id,
                        metadata,
                    })
                    .await;
            }
            Err(error) => {
                let _ = event_tx
                    .send_async(AppEvent::SftpFailed {
                        tab_id,
                        connection_id,
                        op_id,
                        error,
                    })
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
            // 编辑器取全文：与预览同为独立通道，不产生进度事件、不进传输队列。
            // 超时按"上限字节数 + 慢链路"放宽，既要能兜住永久挂起的任务，
            // 也不能让一次正常的慢速读取被误判失败。
            let result =
                tokio::time::timeout(TEXT_READ_TIMEOUT, client.read_text(&path, max_bytes)).await;
            match result {
                Ok(Ok((content, meta))) => {
                    let _ = event_tx
                        .send_async(AppEvent::SftpTextRead {
                            tab_id,
                            connection_id,
                            op_id,
                            content,
                            meta,
                        })
                        .await;
                }
                Ok(Err(error)) => {
                    let _ = event_tx
                        .send_async(AppEvent::SftpTextFailed {
                            tab_id,
                            connection_id,
                            op_id,
                            error,
                        })
                        .await;
                }
                Err(_) => {
                    let _ = event_tx
                        .send_async(AppEvent::SftpTextFailed {
                            tab_id,
                            connection_id,
                            op_id,
                            error: AppError::with_detail(
                                ErrorCode::SftpFailed,
                                format!("读取远端文本超时（{} 秒）", TEXT_READ_TIMEOUT.as_secs()),
                            ),
                        })
                        .await;
                }
            }
        }
        SftpOperation::WriteText {
            path,
            content,
            expected,
        } => {
            // 回传比读取更慢（写要落盘并等待远端确认），给更宽的超时；
            // 失败与超时统一回抛给编辑面板就地显示，不影响队列与状态栏。
            // 版本指纹冲突走独立事件：面板要弹出三选一横幅，不能按普通失败处理。
            let result = tokio::time::timeout(
                TEXT_WRITE_TIMEOUT,
                client.write_text(&path, &content, expected),
            )
            .await;
            match result {
                Ok(Ok(meta)) => {
                    let _ = event_tx
                        .send_async(AppEvent::SftpTextWritten {
                            tab_id,
                            connection_id,
                            op_id,
                            meta,
                        })
                        .await;
                }
                Ok(Err(error)) if error.code == ErrorCode::SftpTextConflict => {
                    let _ = event_tx
                        .send_async(AppEvent::SftpTextConflict {
                            tab_id,
                            connection_id,
                            op_id,
                            error,
                        })
                        .await;
                }
                Ok(Err(error)) => {
                    let _ = event_tx
                        .send_async(AppEvent::SftpTextFailed {
                            tab_id,
                            connection_id,
                            op_id,
                            error,
                        })
                        .await;
                }
                Err(_) => {
                    let _ = event_tx
                        .send_async(AppEvent::SftpTextFailed {
                            tab_id,
                            connection_id,
                            op_id,
                            error: AppError::with_detail(
                                ErrorCode::SftpFailed,
                                format!("回传远端文本超时（{} 秒）", TEXT_WRITE_TIMEOUT.as_secs()),
                            ),
                        })
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
                    .send_async(AppEvent::SftpDirCompared {
                        tab_id,
                        connection_id,
                        op_id,
                        comparison,
                    })
                    .await;
            }
            Err(error) => {
                let _ = event_tx
                    .send_async(AppEvent::SftpFailed {
                        tab_id,
                        connection_id,
                        op_id,
                        error,
                    })
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
    }
}

/// SFTP 命令的调度参数：主循环状态表的借用与命令载荷。
pub(super) struct SftpDispatch<'a> {
    pub(super) remotes: &'a mut HashMap<TabId, (ConnectionId, SshRemote)>,
    pub(super) sftp_clients: &'a mut HashMap<TabId, SftpClient>,
    pub(super) sftp_transfers: &'a mut HashMap<(TabId, TransferId), tokio::task::JoinHandle<()>>,
    pub(super) event_tx: flume::Sender<AppEvent>,
    pub(super) tab_id: TabId,
    pub(super) connection_id: ConnectionId,
    pub(super) op_id: TransferId,
    pub(super) op: SftpOperation,
}

/// SFTP 命令调度：校验连接归属，复用或新建 SFTP 客户端后派发执行。
/// 非就绪或连接已切换直接回失败事件；传输类操作纳入任务表跟踪以便取消。
pub(super) async fn dispatch_sftp_op(dispatch: SftpDispatch<'_>) {
    let SftpDispatch {
        remotes,
        sftp_clients,
        sftp_transfers,
        event_tx,
        tab_id,
        connection_id,
        op_id,
        op,
    } = dispatch;
    let Some((current_connection, remote)) = remotes.get(&tab_id).cloned() else {
        let _ = event_tx
            .send_async(AppEvent::SftpFailed {
                tab_id,
                connection_id,
                op_id,
                error: AppError::with_detail(ErrorCode::ChannelFailed, "SSH 连接尚未就绪"),
            })
            .await;
        return;
    };
    if current_connection != connection_id {
        let _ = event_tx
            .send_async(AppEvent::SftpFailed {
                tab_id,
                connection_id,
                op_id,
                error: AppError::with_detail(ErrorCode::ChannelFailed, "SSH 连接已被新连接替换"),
            })
            .await;
        return;
    }

    let client = if let Some(client) = sftp_clients.get(&tab_id) {
        client.clone()
    } else {
        match SftpClient::open(&remote).await {
            Ok(client) => {
                sftp_clients.insert(tab_id, client.clone());
                client
            }
            Err(error) => {
                let _ = event_tx
                    .send_async(AppEvent::SftpFailed {
                        tab_id,
                        connection_id,
                        op_id,
                        error,
                    })
                    .await;
                return;
            }
        }
    };

    let is_transfer = matches!(
        op,
        SftpOperation::Download { .. }
            | SftpOperation::Upload { .. }
            | SftpOperation::DownloadDir { .. }
            | SftpOperation::UploadDir { .. }
            | SftpOperation::SyncDirs { .. }
    );
    // 直接 spawn 具名 async 函数返回的 future：经由 async 块包装会触发
    // rustc 对泛型闭包的 HRTT 推断限制（FnOnce is not general enough）
    let handle = tokio::spawn(execute_sftp_op(
        event_tx,
        tab_id,
        connection_id,
        op_id,
        client,
        op,
    ));
    if is_transfer {
        sftp_transfers.retain(|_, h| !h.is_finished());
        sftp_transfers.insert((tab_id, op_id), handle);
    }
}
