//! 容器管理工作区事件。

use crate::error::AppError;
use crate::ids::{ContainerRequestId, TabId};
use crate::model::container::{
    ComposeProject, ContainerDetails, ContainerEngineInfo, ContainerEngineProfile,
    ContainerFileEntry, ContainerFileSystemChange, ContainerImageDetails, ContainerImageSummary,
    ContainerNetworkDetails, ContainerNetworkSummary, ContainerProcess, ContainerPruneReport,
    ContainerStats, ContainerSummary, ContainerSystemDiskUsage, ContainerVolumeDetails,
    ContainerVolumeSummary, EngineEvent,
};
use crate::transport::TerminalIo;

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
    /// 容器文件系统变更（Diff）结果。
    ContainerDiffed {
        request_id: ContainerRequestId,
        container: String,
        changes: Vec<ContainerFileSystemChange>,
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
    /// 全局磁盘空间占用查询结果。
    SystemDfLoaded {
        request_id: ContainerRequestId,
        usage: ContainerSystemDiskUsage,
    },
    /// 系统级联合清理完成的结果。
    SystemPruned {
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
    /// 收到容器引擎长连接实时事件。
    EngineEventReceived {
        tab_id: TabId,
        event: EngineEvent,
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
    PodsListed {
        request_id: ContainerRequestId,
        pods: Vec<crate::model::container::ContainerPodSummary>,
    },
    QuadletsListed {
        request_id: ContainerRequestId,
        units: Vec<crate::model::container::ContainerQuadletUnit>,
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
        /// 是否包含 NUL 或非法 UTF-8；二进制文件不允许文本编辑。
        binary: bool,
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
    /// 远端镜像检索完成。
    ImagesSearched {
        request_id: ContainerRequestId,
        results: Vec<crate::model::container::ImageSearchResult>,
    },
    /// 指定镜像仓库的实时标签列表读取完成。
    ImageTagsListed {
        request_id: ContainerRequestId,
        tags: Vec<String>,
    },
    /// 镜像拉取任务分层进度更新。
    PullTaskProgress {
        task_id: String,
        status: String,
        layers: Vec<crate::model::container::ImageLayerProgress>,
    },
    /// 镜像拉取任务结束；`error` 为 `None` 表示成功。
    PullTaskFinished {
        task_id: String,
        error: Option<AppError>,
    },
    /// 列表、发现、预览等非传输类操作失败。
    Failed {
        request_id: ContainerRequestId,
        error: AppError,
    },
}
