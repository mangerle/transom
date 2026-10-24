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
    DeployPublicKey {
        tab_id: TabId,
        connection_id: ConnectionId,
        key_name: String,
        public_key: String,
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
    /// 悬停预览读取完成：`bytes` 为文本文件头部或完整图片内容。
    SftpPreview {
        tab_id: TabId,
        connection_id: ConnectionId,
        op_id: TransferId,
        bytes: Vec<u8>,
        /// 是否因达到读取上限而截断（仅文本头部读取可能出现）。
        truncated: bool,
    },
    /// 悬停预览读取失败；与传输失败分开，避免把轻量预览错误写进面板错误栏。
    SftpPreviewFailed {
        tab_id: TabId,
        connection_id: ConnectionId,
        op_id: TransferId,
        error: AppError,
    },
    /// 编辑器读取远端文本完成。
    SftpTextRead {
        tab_id: TabId,
        connection_id: ConnectionId,
        op_id: TransferId,
        content: String,
    },
    /// 编辑器回传远端文本完成；`size` 为写完后重新 stat 到的远端字节数。
    SftpTextWritten {
        tab_id: TabId,
        connection_id: ConnectionId,
        op_id: TransferId,
        size: u64,
    },
    /// 编辑器读写失败；与 `SftpFailed` 分开，编辑面板的错误不进传输队列与状态栏。
    SftpTextFailed {
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
    /// 按固定周期采样得到的上下行速率：SSH 来自传输层计数，本地终端来自
    /// 本机非回环网卡的累计字节差。
    NetworkTrafficUpdated {
        tab_id: TabId,
        connection_id: ConnectionId,
        traffic: NetworkTraffic,
    },
    /// 公钥部署结果；`error` 为 `None` 表示已成功写入远端 `authorized_keys`。
    PublicKeyDeployed {
        tab_id: TabId,
        connection_id: ConnectionId,
        key_name: String,
        error: Option<AppError>,
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
