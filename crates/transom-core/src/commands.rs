use crate::error::AppError;
use crate::forward::{ForwardRule, ForwardStatus};
use crate::ids::{ConnectionId, ForwardId, TabId, TransferId};
use crate::prompt::PromptInfo;
use crate::secrets::ConnectionSecrets;
use crate::session::Session;
use crate::sftp::{RemoteFile, RemoteMetadata, SftpOperation};
use crate::sftp_sync::DirComparison;
use crate::terminal::{TerminalIo, TerminalSize};
use crate::traffic::NetworkTraffic;

pub enum AppCommand {
    Connect {
        tab_id: TabId,
        connection_id: ConnectionId,
        session: Box<Session>,
        size: TerminalSize,
        secrets: ConnectionSecrets,
    },
    Sftp {
        tab_id: TabId,
        connection_id: ConnectionId,
        op_id: TransferId,
        op: SftpOperation,
    },
    SftpCancel {
        tab_id: TabId,
        op_id: TransferId,
    },
    ForwardStart {
        tab_id: TabId,
        connection_id: ConnectionId,
        rule: ForwardRule,
    },
    ForwardStop {
        tab_id: TabId,
        connection_id: ConnectionId,
        rule_id: ForwardId,
    },
    CloseTab {
        tab_id: TabId,
    },
    Shutdown,
}

pub enum AppEvent {
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
        responder: flume::Sender<Vec<String>>,
    },
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
    },
    SftpFinished {
        tab_id: TabId,
        connection_id: ConnectionId,
        op_id: TransferId,
        transferred: Option<u64>,
        /// 目录传输中被排除规则或符号链接跳过的条目数；单文件操作为 0。
        skipped: u64,
        /// 是否请求并完成了 SHA256 校验。
        verified: bool,
    },
    SftpMetadata {
        tab_id: TabId,
        connection_id: ConnectionId,
        op_id: TransferId,
        metadata: RemoteMetadata,
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
    ForwardChanged {
        tab_id: TabId,
        connection_id: ConnectionId,
        status: ForwardStatus,
    },
    SystemStatsUpdated {
        tab_id: TabId,
        connection_id: ConnectionId,
        stats: SystemStats,
    },
    /// SSH 传输层按固定周期采样得到的上下行速率。
    NetworkTrafficUpdated {
        tab_id: TabId,
        connection_id: ConnectionId,
        traffic: NetworkTraffic,
    },
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct SystemStats {
    pub cpu_percent: f32,
    pub mem_used_bytes: u64,
    pub mem_total_bytes: u64,
    pub disk_used_bytes: u64,
    pub disk_total_bytes: u64,
    pub uptime_secs: u64,
    pub os_name: String,
}
