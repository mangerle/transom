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
use crate::ids::{ConnectionId, ContainerRequestId, SessionId, TabId, TransferId};
use crate::model::container::{
    ComposeProject, ContainerDetails, ContainerEngineInfo, ContainerEngineProfile,
    ContainerFileEntry, ContainerImageDetails, ContainerImageSummary, ContainerNetworkDetails,
    ContainerNetworkSummary, ContainerProcess, ContainerPruneReport, ContainerStats,
    ContainerSummary, ContainerVolumeDetails, ContainerVolumeSummary,
};
use crate::model::forward::ForwardStatus;
use crate::model::host::HostOutcome;
use crate::prompt::PromptInfo;
use crate::protocol::sftp::{
    RemoteFile, RemoteMetadata, RemoteSearchResultItem, RemoteTextMeta, SudoCapability,
};
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
    /// 传输完成、进入完整性校验阶段。
    SftpVerifying {
        tab_id: TabId,
        connection_id: ConnectionId,
        op_id: TransferId,
    },
    /// 校验不一致后的自动恢复：丢弃目标端内容，从头覆盖重传。
    ///
    /// 设计原理（Why）：校验的意义是“发现不一致并恢复”，不是只报一个错让
    /// 用户手点重试。续传前缀可能因为源文件在中断期间被修改而失真，此时唯一
    /// 正确的恢复方式是从头覆盖；把它做成独立事件，界面才能把任务从
    /// “校验中”切回“传输中”并把进度归零，而不是停在一条自相矛盾的文案上。
    SftpRetrying {
        tab_id: TabId,
        connection_id: ConnectionId,
        op_id: TransferId,
        /// 触发重传的不一致原因，供状态栏与日志定位。
        reason: String,
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
    /// 远端提权能力探测结果：界面据此决定是弹口令框、直接提交写入，还是就地说明
    /// 为什么提权不可用。
    SudoProbed {
        tab_id: TabId,
        connection_id: ConnectionId,
        op_id: TransferId,
        capability: SudoCapability,
    },
    SftpDirCompared {
        tab_id: TabId,
        connection_id: ConnectionId,
        op_id: TransferId,
        comparison: DirComparison,
    },
    SftpSearchResults {
        tab_id: TabId,
        connection_id: ConnectionId,
        op_id: TransferId,
        items: Vec<RemoteSearchResultItem>,
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
        stats: Box<super::SystemStats>,
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
    /// 折叠态下的进程总数采样结果。
    ProcessCountSampled {
        tab_id: TabId,
        connection_id: ConnectionId,
        count: u32,
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

/// 容器管理工作区事件。
pub enum ContainerAppEvent {
    /// 引擎连接成功：工作区据此从“连接中”进入可用状态。
    Connected {
        tab_id: TabId,
        info: Box<ContainerEngineInfo>,
    },
    ConnectFailed {
        tab_id: TabId,
        error: AppError,
    },
    /// 容器表单测试连接结果。
    TestFinished {
        request_id: ContainerRequestId,
        info: Option<Box<ContainerEngineInfo>>,
        error: Option<AppError>,
    },
    /// 引擎发现结果：已探测可用的引擎配置与不可用候选的中文摘要。
    EnginesDiscovered {
        request_id: ContainerRequestId,
        engines: Vec<ContainerEngineProfile>,
        failures: Vec<String>,
    },
    Disconnected {
        tab_id: TabId,
    },
    ContainersListed {
        request_id: ContainerRequestId,
        containers: Vec<ContainerSummary>,
    },
    ContainerInspected {
        request_id: ContainerRequestId,
        details: Box<ContainerDetails>,
    },
    ImagesListed {
        request_id: ContainerRequestId,
        images: Vec<ContainerImageSummary>,
    },
    ImageInspected {
        request_id: ContainerRequestId,
        details: Box<ContainerImageDetails>,
    },
    VolumesListed {
        request_id: ContainerRequestId,
        volumes: Vec<ContainerVolumeSummary>,
    },
    VolumeInspected {
        request_id: ContainerRequestId,
        details: Box<ContainerVolumeDetails>,
    },
    /// 清理未使用数据卷的结果：删了哪些、回收多少，界面要如实回报。
    VolumesPruned {
        request_id: ContainerRequestId,
        report: ContainerPruneReport,
    },
    /// 清理悬空镜像的结果。
    ImagesPruned {
        request_id: ContainerRequestId,
        report: ContainerPruneReport,
    },
    /// 清理未使用网络的结果。
    NetworksPruned {
        request_id: ContainerRequestId,
        report: ContainerPruneReport,
    },
    NetworksListed {
        request_id: ContainerRequestId,
        networks: Vec<ContainerNetworkSummary>,
    },
    NetworkInspected {
        request_id: ContainerRequestId,
        details: Box<ContainerNetworkDetails>,
    },
    ComposeProjectsListed {
        request_id: ContainerRequestId,
        projects: Vec<ComposeProject>,
    },
    /// 交互式终端已就绪；UI 用返回的 IO 在详情页签内创建终端视图。
    /// `exit` 在 exec 结束时触发，用于把页签标记为已断开。
    ExecReady {
        tab_id: TabId,
        request_id: ContainerRequestId,
        io: Box<TerminalIo>,
        exit: tokio::sync::oneshot::Receiver<()>,
    },
    /// 日志原始字节（保留 ANSI 着色）；非 TTY 容器的 stderr 已加前缀。
    LogsChunk {
        request_id: ContainerRequestId,
        bytes: Vec<u8>,
    },
    LogsClosed {
        request_id: ContainerRequestId,
    },
    StatsUpdated {
        request_id: ContainerRequestId,
        container: String,
        stats: ContainerStats,
    },
    /// 统计流暂时不可用；UI 保留上一帧并显示自动重连状态。
    StatsUnavailable {
        request_id: ContainerRequestId,
        error: String,
    },
    ProcessesListed {
        request_id: ContainerRequestId,
        processes: Vec<ContainerProcess>,
    },
    DirListed {
        request_id: ContainerRequestId,
        path: String,
        entries: Vec<ContainerFileEntry>,
    },
    /// 文件预览已按 UTF-8 有损解码；`truncated` 表示是否因上限截断。
    FilePreview {
        request_id: ContainerRequestId,
        path: String,
        content: String,
        truncated: bool,
        size: u64,
    },
    TransferProgress {
        request_id: ContainerRequestId,
        transferred: u64,
        total: Option<u64>,
    },
    /// 镜像构建输出行（实时显示当前构建步骤或日志）。
    BuildProgress {
        request_id: ContainerRequestId,
        line: String,
    },
    /// 启停删、拉取、卷/网络操作完成；`error` 为 `None` 表示成功。
    ActionFinished {
        request_id: ContainerRequestId,
        error: Option<AppError>,
    },
    /// 上传/下载完成；`error` 为 `None` 表示成功。
    TransferFinished {
        request_id: ContainerRequestId,
        error: Option<AppError>,
    },
    /// 列表、发现、预览等非传输类操作失败。
    Failed {
        request_id: ContainerRequestId,
        error: AppError,
    },
}

/// 外部唤起请求。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeepLinkRequest {
    Ssh {
        url: String,
    },
    Dir {
        path: PathBuf,
    },
    /// 在本地终端里直接执行命令（`X-TerminalArgExec` / `-e`）。
    ///
    /// `dir` 是可选工作目录：`xdg-terminal-exec` 会把 `--dir=` 与 `-e` 一起
    /// 传入，两条信息必须随同一个请求到达，否则会先开一个多余的 Shell 标签。
    Exec {
        command: Vec<String>,
        dir: Option<PathBuf>,
    },
    Activate,
}
