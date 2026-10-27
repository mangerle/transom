//! SFTP 传输任务与调度状态模型。

use crate::ids::{ConnectionId, TransferId};
use crate::sftp_sync::SyncItem;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransferDirection {
    Download,
    Upload,
    /// 目录双向增量同步：单个任务内同时包含上传与下载项
    Sync,
    /// 跨远端流式互传（两台远端服务器之间流式复制）
    RemoteCopy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TransferKind {
    #[default]
    File,
    Directory,
    /// 目录对比后按差异计划双向同步
    Sync,
}

/// 目录递归传输的汇总结果。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TransferSummary {
    /// 实际完成（含断点续传已存在）的字节数
    pub transferred: u64,
    /// 被排除规则或符号链接跳过的条目数
    pub skipped: u64,
    /// 是否请求了 SHA256 校验且全部文件校验通过
    pub verified: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransferStatus {
    Pending,
    Transferring,
    Paused,
    Completed,
    Failed(String),
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferTask {
    pub id: TransferId,
    pub direction: TransferDirection,
    #[serde(default)]
    pub kind: TransferKind,
    pub local_path: PathBuf,
    pub remote_path: String,
    /// 跨远端互传时的源端连接 ID；本地上传/下载时为 None。
    #[serde(default)]
    pub source_connection_id: Option<ConnectionId>,
    /// 跨远端互传时的目标端连接 ID；本地上传/下载时为 None。
    #[serde(default)]
    pub target_connection_id: Option<ConnectionId>,
    /// 跨远端互传时的目标端绝对路径。
    #[serde(default)]
    pub target_remote_path: Option<String>,
    pub size: Option<u64>,
    pub transferred: u64,
    /// 实时传输平滑速率（字节/秒）。
    #[serde(default)]
    pub speed_bps: Option<u64>,
    /// 预计剩余秒数。
    #[serde(default)]
    pub eta_secs: Option<u64>,
    pub status: TransferStatus,
    pub mode: TransferMode,
    /// 同步任务携带的差异项；其他任务为空。
    #[serde(default)]
    pub sync_items: Vec<SyncItem>,
    /// 任务入队时是否要求传输完成后做 SHA256 校验。
    #[serde(default)]
    pub verify: bool,
    /// 是否在传输完成后保留原始文件的 POSIX 权限与修改时间戳。
    #[serde(default = "default_true")]
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
