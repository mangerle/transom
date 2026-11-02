//! SFTP 队列任务与调度状态模型。
//!
//! 设计原理（Why）：`TransferTask` 是界面队列的唯一事实源——入队、并发调度、
//! 暂停/恢复、进度展示都围绕它展开，而控制层只接收离散的 `SftpOperation`
//! 命令，从不读取队列。把它留在 core 会让"纯契约层"背上只有 UI 使用的状态机；
//! 迁到 UI 后，core 只保留跨层的传输方向与目录汇总结果。
//!
//! 传输起点也由本模型持有：新任务恒为覆盖，只有被中断且仍保留进度的任务才会
//! 声明续传偏移。这样“目标端已存在同名文件”永远不会被误当成断点。

use std::path::PathBuf;
use transom_core::{
    AppError, ConnectionId, ErrorCode, SyncItem, TabId, TransferDirection, TransferId,
    TransferStart, VerifyOptions,
};

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

/// 传输失败的场合分类。
///
/// 设计原理（Why）：重试从哪里开始完全取决于失败的性质——校验认定目标端内容
/// 不可信时必须从头覆盖，普通中断则可以从已写入的字节继续。此前用“错误文案
/// 里是否出现校验字样”来猜，既依赖本地化文案，又会把“续传后校验不一致”与
/// “真正的传输中断”混为一谈；把分类放进状态，重试路径只做类型分支。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransferFailure {
    /// 传输后完整性校验不一致：目标端内容不可信，重试必须从头覆盖。
    Verify(String),
    /// 传输中途中断：目标端已写入的部分来自本任务，可按进度续传。
    Interrupted(String),
}

impl TransferFailure {
    /// 面向用户的失败原因。
    pub(crate) fn reason(&self) -> &str {
        match self {
            Self::Verify(reason) | Self::Interrupted(reason) => reason,
        }
    }

    /// 按错误码归类失败场合。
    pub(crate) fn from_error(error: &AppError, reason: String) -> Self {
        if error.code == ErrorCode::SftpVerifyFailed {
            Self::Verify(reason)
        } else {
            Self::Interrupted(reason)
        }
    }

    /// 该失败是否要求重试从第 0 字节开始。
    pub(crate) fn requires_full_restart(&self) -> bool {
        matches!(self, Self::Verify(_))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransferStatus {
    Pending,
    Transferring,
    Verifying,
    Paused,
    Completed,
    Failed(TransferFailure),
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
    /// 下一次执行时的传输起点：新任务恒为覆盖，仅被中断的任务可续传。
    pub start: TransferStart,
    /// 同步任务携带的差异项；其他任务为空。
    pub sync_items: Vec<SyncItem>,
    /// 入队时固化的传输后校验选项（是否校验、策略与算法）。
    pub verify: VerifyOptions,
    /// 是否在传输完成后保留原始文件的 POSIX 权限与修改时间戳。
    pub preserve_attributes: bool,
}

impl TransferTask {
    pub fn new(
        direction: TransferDirection,
        local_path: PathBuf,
        remote_path: String,
        size: Option<u64>,
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
            start: TransferStart::Overwrite,
            sync_items: Vec::new(),
            verify: VerifyOptions::default(),
            preserve_attributes: true,
        }
    }

    /// 创建目录递归传输任务（队列中一个任务代表整棵目录树）。
    pub fn new_directory(
        direction: TransferDirection,
        local_path: PathBuf,
        remote_path: String,
        size: Option<u64>,
    ) -> Self {
        let mut task = Self::new(direction, local_path, remote_path, size);
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
        let mut task = Self::new(TransferDirection::Sync, local_root, remote_root, size);
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
        );
        if is_dir {
            task.kind = TransferKind::Directory;
        }
        task.source_connection_id = Some(source_connection_id);
        task.target_connection_id = Some(target_connection_id);
        task.target_remote_path = Some(target_path);
        task
    }

    /// 从上次中断处继续：仅当启用断点续传且仍有进度时使用记录偏移，
    /// 否则丢弃进度从头覆盖（用户关掉断点续传即表示接受重新传输）。
    pub(crate) fn plan_resume(&mut self, resume_enabled: bool) {
        if resume_enabled && self.transferred > 0 {
            self.start = TransferStart::Continue {
                offset: self.transferred,
            };
        } else {
            self.plan_restart();
        }
    }

    /// 丢弃已有进度，从头覆盖重传（校验不一致或用户明确要求重来）。
    pub(crate) fn plan_restart(&mut self) {
        self.start = TransferStart::Overwrite;
        self.transferred = 0;
    }

    /// 重新排队为待处理，并按当前状态决定起点。
    ///
    /// 校验不一致、已完成与已取消的任务都没有可复用的可信前缀，必须从头覆盖；
    /// 暂停、在途（含被“置顶”打断）与待处理任务保留进度，续传是否生效由
    /// 断点续传开关决定。重排入口共用这一判断，避免每个按钮各写一套条件。
    pub(crate) fn requeue(&mut self, resume_enabled: bool) {
        let restart = match &self.status {
            TransferStatus::Failed(failure) => failure.requires_full_restart(),
            TransferStatus::Completed | TransferStatus::Cancelled => true,
            _ => false,
        };
        self.status = TransferStatus::Pending;
        if restart {
            self.plan_restart();
        } else {
            self.plan_resume(resume_enabled);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{TransferFailure, TransferKind, TransferStart, TransferStatus, TransferTask};
    use transom_core::{AppError, ConnectionId, ErrorCode, TransferDirection};

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

    /// 新建任务永远是覆盖起点：目标端已有同名文件不能当成断点。
    #[test]
    fn new_task_starts_from_overwrite() {
        let task = TransferTask::new(
            TransferDirection::Upload,
            "/tmp/a".into(),
            "/remote/a".into(),
            Some(10),
        );
        assert_eq!(task.start, TransferStart::Overwrite);
        assert_eq!(task.transferred, 0);
    }

    /// 续传只在“启用断点续传且有进度”时生效，否则丢弃进度重新开始。
    #[test]
    fn resume_requires_progress_and_setting() {
        let mut task = TransferTask::new(
            TransferDirection::Download,
            "/tmp/a".into(),
            "/remote/a".into(),
            Some(100),
        );
        task.transferred = 40;
        task.plan_resume(true);
        assert_eq!(task.start, TransferStart::Continue { offset: 40 });

        task.plan_resume(false);
        assert_eq!(task.start, TransferStart::Overwrite);
        assert_eq!(task.transferred, 0);
    }

    /// 校验失败归类为“必须从头覆盖”，普通失败属于可续传的中断。
    #[test]
    fn failure_kind_decides_restart() {
        let verify_error = AppError::new(ErrorCode::SftpVerifyFailed);
        let failure = TransferFailure::from_error(&verify_error, "校验失败".into());
        assert!(failure.requires_full_restart());
        assert_eq!(failure.reason(), "校验失败");

        let interrupted =
            TransferFailure::from_error(&AppError::new(ErrorCode::SftpFailed), "连接中断".into());
        assert!(!interrupted.requires_full_restart());
        assert_eq!(interrupted.reason(), "连接中断");
    }
}
