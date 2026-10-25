use crate::error::AppError;
use crate::forward::{ForwardRule, ForwardStatus};
use crate::ids::{ConnectionId, ForwardId, TabId, TransferId};
use crate::prompt::PromptInfo;
use crate::secrets::ConnectionSecrets;
use crate::session::Session;
use crate::sftp::{RemoteFile, RemoteMetadata, RemoteTextMeta, SftpOperation};
use crate::sftp_sync::DirComparison;
use crate::sync::{SyncPushReport, SyncSettings};
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
    /// 把目录快照加密上传到云同步端点。
    ///
    /// `catalog` 是界面侧目录的完整快照：同步不在控制器内维护目录状态，
    /// 快照进快出，控制平面不感知会话内容的生命周期。
    CloudSyncPush {
        settings: SyncSettings,
        catalog: crate::catalog::SessionCatalog,
    },
    /// 从云同步端点下载并解密备份文档，合并动作由界面侧完成。
    CloudSyncPull {
        settings: SyncSettings,
    },
    /// ZMODEM 上传决策：用户从文件选择对话框确认后的文件列表。
    ZmodemUpload {
        tab_id: TabId,
        files: Vec<std::path::PathBuf>,
    },
    /// 取消当前标签的 ZMODEM 传输（状态栏按钮或 Esc）。
    ZmodemCancel {
        tab_id: TabId,
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
        responder: tokio::sync::mpsc::Sender<Vec<String>>,
    },
    /// 首次连接遇到未知主机密钥，请求用户确认是否信任该指纹（TOFU）。
    /// `responder` 回传用户决定；弹窗被关闭时响应通道析构，认证方按取消处理。
    HostKeyConfirm {
        tab_id: TabId,
        connection_id: ConnectionId,
        host: String,
        port: u16,
        key_type: String,
        fingerprint: String,
        responder: tokio::sync::mpsc::Sender<bool>,
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
    /// 编辑器读取远端文本完成；`meta` 为读取时刻的版本指纹，随内容一并缓存
    /// 到编辑面板，供保存前的并发冲突比对。
    SftpTextRead {
        tab_id: TabId,
        connection_id: ConnectionId,
        op_id: TransferId,
        content: String,
        meta: RemoteTextMeta,
    },
    /// 编辑器回传远端文本完成；`meta` 为写完后重新 stat 到的版本指纹与字节数，
    /// 作为新的比对基准回填面板。
    SftpTextWritten {
        tab_id: TabId,
        connection_id: ConnectionId,
        op_id: TransferId,
        meta: RemoteTextMeta,
    },
    /// 编辑器读写失败；与 `SftpFailed` 分开，编辑面板的错误不进传输队列与状态栏。
    SftpTextFailed {
        tab_id: TabId,
        connection_id: ConnectionId,
        op_id: TransferId,
        error: AppError,
    },
    /// 回传前的版本指纹比对发现远端已被修改，写入被拒绝；编辑面板据此弹出
    /// 冲突横幅（重新载入 / 强制覆盖 / 继续编辑），而非按普通失败处理。
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
    /// 云端上传完成；`Err` 时界面展示错误，成功时回填统计与时间戳。
    CloudSyncPushed {
        result: Result<SyncPushReport, AppError>,
    },
    /// 云端下载解密完成；备份文档交由界面合并，口令写回也在界面侧编排。
    CloudSyncPulled {
        result: Result<crate::backup::BackupDocument, AppError>,
    },
    /// ZMODEM 传输事件（开始/进度/请求选择文件/结束），由控制器从传输层
    /// 事件通道转发。
    Zmodem {
        tab_id: TabId,
        connection_id: ConnectionId,
        event: crate::zmodem::ZmodemEvent,
    },
    /// 桌面深链唤起：`Some(url)` 是外部（浏览器 `ssh://` 链接、其他工具）
    /// 请求直连的地址；`None` 表示用户在没有地址的情况下再次启动应用，
    /// 只需要把已有实例唤起。经单实例套接字进入与控制器共用的事件通道。
    DeepLink { url: Option<String> },
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
