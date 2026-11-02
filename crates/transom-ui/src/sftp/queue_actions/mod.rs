//! SFTP 传输队列调度、入队与批量控制。
//!
//! 设计原理（Why）：
//! - 实现初衷：负责 SFTP 传输队列的并发调度、任务分发与整列批量控制（暂停全部、恢复全部、取消全部）；
//! - 核心优势：
//!   1. 集中管理并发调度循环 `process_sftp_queue`；
//!   2. 规范化整批操作日志与命令发送顺序。

mod download;
pub(super) mod enqueue;
mod task_control;

use crate::app::AppShell;
use crate::sftp::{TransferKind, TransferTask};
use gpui_kit::*;
use std::path::PathBuf;
use transom_core::{ConnectionId, SftpOperation, TabId, TransferDirection, TransferFilter};

impl AppShell {
    /// 把本地列选中的一组路径上传到标签页主远端当前目录。
    pub(crate) fn enqueue_uploads(
        &mut self,
        tab_id: TabId,
        paths: &[PathBuf],
        cx: &mut Context<Self>,
    ) -> Result<usize, String> {
        let Some(view) = self.state.sftp_view(tab_id) else {
            return Err(rust_i18n::t!("sftp.queue_actions.no_remote_upload").to_string());
        };
        view.update(cx, |view, cx| view.enqueue_uploads(tab_id, paths, cx))
    }

    /// 按并发上限启动待处理任务；调度本体在实体上，外壳只负责转发。
    pub(crate) fn process_sftp_queue(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if let Some(view) = self.state.sftp_view(tab_id) {
            view.update(cx, |view, cx| view.process_queue(tab_id, cx));
        }
    }

    /// 暂停全部活动与排队任务；队列收起时由状态栏调用。
    pub(crate) fn sftp_pause_all(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if let Some(view) = self.state.sftp_view(tab_id) {
            view.update(cx, |view, cx| view.pause_all(tab_id, cx));
        }
    }

    /// 取消全部活动、排队与暂停任务。
    pub(crate) fn sftp_cancel_all(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if let Some(view) = self.state.sftp_view(tab_id) {
            view.update(cx, |view, cx| view.cancel_all(tab_id, cx));
        }
    }
}

/// 任务执行所需的远端连接；方向决定用哪一端，缺失即视为入队缺陷。
///
/// 不再回退标签主连接或第一列：那会把跨列传输静默送到错误主机。
pub(super) fn task_connection_id(task: &TransferTask) -> Option<ConnectionId> {
    match task.direction {
        TransferDirection::Upload => task.target_connection_id,
        TransferDirection::Download => task.source_connection_id,
        TransferDirection::Sync | TransferDirection::RemoteCopy => {
            task.target_connection_id.or(task.source_connection_id)
        }
    }
}

pub(super) fn sftp_operation_for_task(
    task: &TransferTask,
    filter: &TransferFilter,
) -> Option<SftpOperation> {
    let verify = task.verify;
    let start = task.start;
    let op = match (task.kind, task.direction) {
        (TransferKind::Directory, TransferDirection::Upload) => SftpOperation::UploadDir {
            local_path: task.local_path.clone(),
            remote_path: task.remote_path.clone(),
            filter: filter.clone(),
            verify,
        },
        (TransferKind::Directory, TransferDirection::Download) => SftpOperation::DownloadDir {
            remote_path: task.remote_path.clone(),
            local_path: task.local_path.clone(),
            filter: filter.clone(),
            verify,
        },
        (TransferKind::File, TransferDirection::Upload) => SftpOperation::Upload {
            local_path: task.local_path.clone(),
            remote_path: task.remote_path.clone(),
            start,
            verify,
        },
        (TransferKind::File, TransferDirection::Download) => SftpOperation::Download {
            remote_path: task.remote_path.clone(),
            local_path: task.local_path.clone(),
            start,
            verify,
        },
        (TransferKind::Sync, TransferDirection::Sync) => SftpOperation::SyncDirs {
            local_path: task.local_path.clone(),
            remote_path: task.remote_path.clone(),
            items: task.sync_items.clone(),
            verify,
        },
        (_, TransferDirection::RemoteCopy) => {
            let source_conn = task.source_connection_id?;
            let target_conn = task.target_connection_id?;
            let target_path = task.target_remote_path.clone()?;
            SftpOperation::RemoteTransfer {
                source_connection_id: source_conn,
                source_path: task.remote_path.clone(),
                target_connection_id: target_conn,
                target_path,
                is_dir: task.kind == TransferKind::Directory,
                filter: filter.clone(),
                preserve_attributes: task.preserve_attributes,
            }
        }
        _ => return None,
    };
    Some(op)
}

#[cfg(test)]
mod tests {
    use super::task_connection_id;
    use crate::sftp::TransferTask;
    use std::path::PathBuf;
    use transom_core::{ConnectionId, TransferDirection};

    fn task(direction: TransferDirection) -> TransferTask {
        TransferTask::new(direction, PathBuf::from("/tmp/a"), "/remote/a".into(), None)
    }

    /// 上传只认目标端连接：缺失时宁可失败，也不能回退到标签主连接。
    #[test]
    fn upload_requires_target_connection() {
        let mut upload = task(TransferDirection::Upload);
        assert_eq!(task_connection_id(&upload), None);
        let target = ConnectionId::new();
        upload.target_connection_id = Some(target);
        assert_eq!(task_connection_id(&upload), Some(target));
    }

    /// 下载只认源端连接，绝不能拿目标端字段兜底。
    #[test]
    fn download_requires_source_connection() {
        let mut download = task(TransferDirection::Download);
        assert_eq!(task_connection_id(&download), None);
        let source = ConnectionId::new();
        download.source_connection_id = Some(source);
        download.target_connection_id = Some(ConnectionId::new());
        assert_eq!(task_connection_id(&download), Some(source));
    }

    /// 跨远端互传两端都带连接，任一端可用即可调度（操作内部再校验两端）。
    #[test]
    fn remote_copy_accepts_either_endpoint() {
        let mut copy = task(TransferDirection::RemoteCopy);
        let source = ConnectionId::new();
        copy.source_connection_id = Some(source);
        assert_eq!(task_connection_id(&copy), Some(source));
    }
}
