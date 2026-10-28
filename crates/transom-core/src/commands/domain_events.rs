//! 应用分领域事件枚举定义。
//!
//! 将原先平铺在顶层 AppEvent 中的 28 个变体按照业务子系统收敛为内聚的领域事件：
//! - TerminalEvent: 终端连接就绪、建连失败、连接断开、交互式认证提示、未知主机指纹确认 (TOFU)
//! - SftpEvent: 文件列表、传输进度、操作结束、属性元数据、悬停预览、文本读取/写回/冲突/失败、目录比较
//! - ForwardEvent: 端口转发状态变更
//! - HostEvent: 主机拨测/Wake-on-LAN 结局回报、公钥部署结果、进程快照采样与进程终止结果
//! - SyncEvent: WebDAV/S3 备份云端上传/下载、Git 冲突三路合并同步结果
//! - ZmodemEvent: ZMODEM 传输协议事件

use crate::error::AppError;
use crate::ids::{ConnectionId, SessionId, TabId, TransferId};
use crate::model::forward::ForwardStatus;
use crate::model::host::HostOutcome;
use crate::prompt::PromptInfo;
use crate::protocol::sftp::{RemoteFile, RemoteMetadata, RemoteTextMeta};
use crate::protocol::sftp_sync::DirComparison;
use crate::protocol::sync::{GitSyncResult, SyncPushReport};
use crate::protocol::system_stats::ProcessInfo;
use crate::protocol::traffic::NetworkTraffic;
use crate::transport::TerminalIo;
use std::path::PathBuf;

/// 终端会话与连接相关的事件。
pub enum TerminalAppEvent {
    TerminalReady {
        tab_id: TabId,
        connection_id: ConnectionId,
        io: TerminalIo,
    },
    ConnectionFailed {
        tab_id: TabId,
        connection_id: ConnectionId,
        error: AppError,
    },
    Disconnected {
        tab_id: TabId,
        connection_id: ConnectionId,
    },
    AuthPrompt {
        tab_id: TabId,
        connection_id: ConnectionId,
        name: String,
        instructions: String,
        prompts: Vec<PromptInfo>,
        responder: tokio::sync::mpsc::Sender<Vec<String>>,
    },
    /// 首次连接遇到未知主机密钥，请求用户确认是否信任该指纹（TOFU）。
    HostKeyConfirm {
        tab_id: TabId,
        connection_id: ConnectionId,
        host: String,
        port: u16,
        key_type: String,
        fingerprint: String,
        responder: tokio::sync::mpsc::Sender<bool>,
    },
}

/// SFTP 文件传输、列表扫描与文本编辑事件。
pub enum SftpAppEvent {
    SftpListing {
        tab_id: TabId,
        connection_id: ConnectionId,
        path: String,
        entries: Vec<RemoteFile>,
    },
    SftpProgress {
        tab_id: TabId,
        connection_id: ConnectionId,
        op_id: TransferId,
        transferred: u64,
        total: Option<u64>,
        speed_bps: Option<u64>,
        eta_secs: Option<u64>,
    },
    SftpFinished {
        tab_id: TabId,
        connection_id: ConnectionId,
        op_id: TransferId,
        transferred: Option<u64>,
        skipped: u64,
        verified: bool,
    },
    SftpMetadata {
        tab_id: TabId,
        connection_id: ConnectionId,
        op_id: TransferId,
        metadata: RemoteMetadata,
    },
    SftpPreview {
        tab_id: TabId,
        connection_id: ConnectionId,
        op_id: TransferId,
        bytes: Vec<u8>,
        truncated: bool,
    },
    SftpPreviewFailed {
        tab_id: TabId,
        connection_id: ConnectionId,
        op_id: TransferId,
        error: AppError,
    },
    SftpTextRead {
        tab_id: TabId,
        connection_id: ConnectionId,
        op_id: TransferId,
        content: String,
        meta: RemoteTextMeta,
    },
    SftpTextWritten {
        tab_id: TabId,
        connection_id: ConnectionId,
        op_id: TransferId,
        meta: RemoteTextMeta,
    },
    SftpTextFailed {
        tab_id: TabId,
        connection_id: ConnectionId,
        op_id: TransferId,
        error: AppError,
    },
    SftpTextConflict {
        tab_id: TabId,
        connection_id: ConnectionId,
        op_id: TransferId,
        error: AppError,
    },
    SftpDirCompared {
        tab_id: TabId,
        connection_id: ConnectionId,
        op_id: TransferId,
        comparison: DirComparison,
    },
    SftpFailed {
        tab_id: TabId,
        connection_id: ConnectionId,
        op_id: TransferId,
        error: AppError,
    },
}

/// 端口转发事件。
pub enum ForwardAppEvent {
    ForwardChanged {
        tab_id: TabId,
        connection_id: ConnectionId,
        status: ForwardStatus,
    },
}

/// 主机监控、遥测、拨测与运维动作事件。
pub enum HostAppEvent {
    SystemStatsUpdated {
        tab_id: TabId,
        connection_id: ConnectionId,
        stats: super::SystemStats,
    },
    NetworkTrafficUpdated {
        tab_id: TabId,
        connection_id: ConnectionId,
        traffic: NetworkTraffic,
    },
    PublicKeyDeployed {
        tab_id: TabId,
        connection_id: ConnectionId,
        key_name: String,
        error: Option<AppError>,
    },
    HostActionResult {
        session_id: SessionId,
        outcome: HostOutcome,
    },
    ProcessesSampled {
        tab_id: TabId,
        connection_id: ConnectionId,
        processes: Vec<ProcessInfo>,
    },
    ProcessesSampleFailed {
        tab_id: TabId,
        connection_id: ConnectionId,
        error: AppError,
    },
    ProcessKilled {
        tab_id: TabId,
        connection_id: ConnectionId,
        pid: u32,
        error: Option<AppError>,
    },
}

/// 云端备份与 Git 同步事件。
pub enum SyncAppEvent {
    CloudSyncPushed {
        result: Result<SyncPushReport, AppError>,
    },
    CloudSyncPulled {
        result: Result<crate::backup::BackupDocument, AppError>,
    },
    GitSynced {
        result: Result<GitSyncResult, AppError>,
    },
}

/// ZMODEM 传输事件。
pub enum ZmodemAppEvent {
    Zmodem {
        tab_id: TabId,
        connection_id: ConnectionId,
        event: crate::protocol::zmodem::ZmodemEvent,
    },
}

/// 外部唤起请求。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeepLinkRequest {
    Ssh { url: String },
    Dir { path: PathBuf },
    Activate,
}
