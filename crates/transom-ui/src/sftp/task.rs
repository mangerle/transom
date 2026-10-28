//! SFTP 队列任务与调度状态模型。
//!
//! 设计原理（Why）：`TransferTask` 是界面队列的唯一事实源——入队、并发调度、
//! 暂停/恢复、进度展示都围绕它展开，而控制层只接收离散的 `SftpOperation`
//! 命令，从不读取队列。把它留在 core 会让"纯契约层"背上只有 UI 使用的状态机；
//! 迁到 UI 后，core 只保留跨层的传输方向与目录汇总结果。

use std::path::PathBuf;
use transom_core::{ConnectionId, SyncItem, TabId, TransferDirection, TransferId};

/// 单条上传/下载入队请求。
///
/// 设计原理（Why）：入队路径曾允许省略连接，调度层再回退到主连接或第一列，
/// 跨列操作会把文件静默送到另一台主机；把方向、连接与条目类型收进一个请求
/// 对象，让"不知道目标连接"在类型层面无法入队，调度层也不再猜测。
pub(crate) struct SftpTransferRequest {
    pub(crate) tab_id: TabId,
    /// 上传时为目标端，下载时为源端。
    pub(crate) connection_id: ConnectionId,
    pub(crate) direction: TransferDirection,
    pub(crate) is_dir: bool,
    pub(crate) local_path: PathBuf,
    pub(crate) remote_path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TransferMode {
    #[default]
    Auto,
    Resume,
    Overwrite,
}

/// 传输目标类型：单文件或整棵目录树。
///
/// 目录传输在队列中只占一个任务，由后台逐项递归执行；`kind` 决定队列调度器
/// 选择单文件操作还是目录递归操作，避免根据路径实时猜测类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TransferKind {
    #[default]
    File,
    Directory,
    /// 目录对比后按差异计划双向同步
    Sync,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransferStatus {
    Pending,
    Transferring,
    Paused,
    Completed,
    Failed(String),
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferTask {
    pub id: TransferId,
    pub direction: TransferDirection,
    pub kind: TransferKind,
    pub local_path: PathBuf,
    pub remote_path: String,
    /// 跨远端互传时的源端连接 ID；本地上传/下载时为 None。
    pub source_connection_id: Option<ConnectionId>,
    /// 跨远端互传时的目标端连接 ID；本地上传/下载时为 None。
    pub target_connection_id: Option<ConnectionId>,
    /// 跨远端互传时的目标端绝对路径。
    pub target_remote_path: Option<String>,
    pub size: Option<u64>,
    pub transferred: u64,
    /// 实时传输平滑速率（字节/秒）。
    pub speed_bps: Option<u64>,
    /// 预计剩余秒数。
    pub eta_secs: Option<u64>,
    pub status: TransferStatus,
    pub mode: TransferMode,
    /// 同步任务携带的差异项；其他任务为空。
    pub sync_items: Vec<SyncItem>,
    /// 任务入队时是否要求传输完成后做 SHA256 校验。
    pub verify: bool,
    /// 是否在传输完成后保留原始文件的 POSIX 权限与修改时间戳。
    pub preserve_attributes: bool,
}

impl TransferTask {
    pub fn new(
        direction: TransferDirection,
        local_path: PathBuf,
        remote_path: String,
        size: Option<u64>,
        mode: TransferMode,
    ) -> Self {
        Self {
            id: TransferId::new(),
            direction,
            kind: TransferKind::File,
            local_path,
            remote_path,
            source_connection_id: None,
            target_connection_id: None,
            target_remote_path: None,
            size,
            transferred: 0,
            speed_bps: None,
            eta_secs: None,
            status: TransferStatus::Pending,
            mode,
            sync_items: Vec::new(),
            verify: false,
            preserve_attributes: true,
        }
    }

    /// 创建目录递归传输任务（队列中一个任务代表整棵目录树）。
    pub fn new_directory(
        direction: TransferDirection,
        local_path: PathBuf,
        remote_path: String,
        size: Option<u64>,
        mode: TransferMode,
    ) -> Self {
        let mut task = Self::new(direction, local_path, remote_path, size, mode);
        task.kind = TransferKind::Directory;
        task
    }

    /// 创建目录双向同步任务：`local_path`/`remote_path` 为两端根目录。
    pub fn new_sync(
        local_root: PathBuf,
        remote_root: String,
        items: Vec<SyncItem>,
        size: Option<u64>,
    ) -> Self {
        let mut task = Self::new(
            TransferDirection::Sync,
            local_root,
            remote_root,
            size,
            TransferMode::Overwrite,
        );
        task.kind = TransferKind::Sync;
        task.sync_items = items;
        task
    }

    /// 创建跨远端互传任务（客户端流式中转）。
    pub fn new_remote_copy(
        source_connection_id: ConnectionId,
        source_path: String,
        target_connection_id: ConnectionId,
        target_path: String,
        size: Option<u64>,
        is_dir: bool,
    ) -> Self {
        let mut task = Self::new(
            TransferDirection::RemoteCopy,
            PathBuf::from(&source_path),
            source_path,
            size,
            TransferMode::Auto,
        );
        if is_dir {
            task.kind = TransferKind::Directory;
        }
        task.source_connection_id = Some(source_connection_id);
        task.target_connection_id = Some(target_connection_id);
        task.target_remote_path = Some(target_path);
        task
    }
}

#[cfg(test)]
mod tests {
    use super::{TransferKind, TransferStatus, TransferTask};
    use transom_core::{ConnectionId, TransferDirection};

    #[test]
    fn remote_copy_task_carries_both_endpoints() {
        let conn_a = ConnectionId::new();
        let conn_b = ConnectionId::new();
        let task = TransferTask::new_remote_copy(
            conn_a,
            "/remote/a/file.txt".into(),
            conn_b,
            "/remote/b/file.txt".into(),
            Some(1024),
            false,
        );
        assert_eq!(task.direction, TransferDirection::RemoteCopy);
        assert_eq!(task.kind, TransferKind::File);
        assert_eq!(task.status, TransferStatus::Pending);
        assert_eq!(task.source_connection_id, Some(conn_a));
        assert_eq!(task.target_connection_id, Some(conn_b));
        assert_eq!(
            task.target_remote_path.as_deref(),
            Some("/remote/b/file.txt")
        );
        assert!(task.preserve_attributes);
        assert_eq!(task.size, Some(1024));
    }
}
