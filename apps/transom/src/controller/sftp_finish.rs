//! SFTP 操作结果的事件回传助手：单值、目录汇总与无返回值三类收尾。

use std::path::Path;
use transom_core::{AppError, AppEvent, ConnectionId, ErrorCode, PreviewMode, TabId, TransferId};
use transom_net::SftpClient;

pub(super) async fn verify_transfer_result(
    client: &SftpClient,
    result: Result<u64, AppError>,
    verify: bool,
    local_path: &Path,
    remote_path: &str,
) -> Result<(u64, bool), AppError> {
    let transferred = result?;
    if verify {
        client.verify_file(local_path, remote_path).await?;
    }
    Ok((transferred, verify))
}

pub(super) async fn finish_sftp(
    event_tx: flume::Sender<AppEvent>,
    tab_id: TabId,
    connection_id: ConnectionId,
    op_id: TransferId,
    result: Result<(u64, bool), AppError>,
) {
    match result {
        Ok((transferred, verified)) => {
            let _ = event_tx
                .send_async(AppEvent::SftpFinished {
                    tab_id,
                    connection_id,
                    op_id,
                    transferred: Some(transferred),
                    skipped: 0,
                    verified,
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
    }
}

pub(super) async fn finish_sftp_summary(
    event_tx: flume::Sender<AppEvent>,
    tab_id: TabId,
    connection_id: ConnectionId,
    op_id: TransferId,
    result: Result<transom_core::TransferSummary, AppError>,
) {
    match result {
        Ok(summary) => {
            let _ = event_tx
                .send_async(AppEvent::SftpFinished {
                    tab_id,
                    connection_id,
                    op_id,
                    transferred: Some(summary.transferred),
                    skipped: summary.skipped,
                    verified: summary.verified,
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
    }
}

pub(super) async fn finish_sftp_none(
    event_tx: flume::Sender<AppEvent>,
    tab_id: TabId,
    connection_id: ConnectionId,
    op_id: TransferId,
    result: Result<(), AppError>,
) {
    match result {
        Ok(()) => {
            let _ = event_tx
                .send_async(AppEvent::SftpFinished {
                    tab_id,
                    connection_id,
                    op_id,
                    transferred: None,
                    skipped: 0,
                    verified: false,
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
    }
}

/// 预览请求的参数收敛：客户端借用、事件通道与命令载荷。
pub(super) struct PreviewRequest<'a> {
    pub(super) client: &'a SftpClient,
    pub(super) event_tx: flume::Sender<AppEvent>,
    pub(super) tab_id: TabId,
    pub(super) connection_id: ConnectionId,
    pub(super) op_id: TransferId,
    pub(super) path: String,
    pub(super) mode: PreviewMode,
    pub(super) max_bytes: u64,
}

/// 悬停预览读取的最长等待时间。
///
/// 预览属于可随时放弃的低优先级只读请求；远端套接字挂起时必须尽快收尾，
/// 否则一次卡顿会让预览通道长期占用内存并阻塞同一会话上的后续预览。
const PREVIEW_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// 预览是低优先级只读请求：远端卡住时超时放弃，避免长时间占用会话与
/// 内存；成功与失败都走独立事件，不污染传输队列和状态栏。
pub(super) async fn read_preview(request: PreviewRequest<'_>) {
    let PreviewRequest {
        client,
        event_tx,
        tab_id,
        connection_id,
        op_id,
        path,
        mode,
        max_bytes,
    } = request;
    let result =
        tokio::time::timeout(PREVIEW_TIMEOUT, client.read_preview(&path, mode, max_bytes)).await;
    match result {
        Ok(Ok((bytes, truncated))) => {
            let _ = event_tx
                .send_async(AppEvent::SftpPreview {
                    tab_id,
                    connection_id,
                    op_id,
                    bytes,
                    truncated,
                })
                .await;
        }
        Ok(Err(error)) => {
            let _ = event_tx
                .send_async(AppEvent::SftpFailed {
                    tab_id,
                    connection_id,
                    op_id,
                    error,
                })
                .await;
        }
        Err(_) => {
            let _ = event_tx
                .send_async(AppEvent::SftpFailed {
                    tab_id,
                    connection_id,
                    op_id,
                    error: AppError::with_detail(
                        ErrorCode::Timeout,
                        format!("预览读取超时（超过 10 秒）: {path}"),
                    ),
                })
                .await;
        }
    }
}
