//! 容器引擎工作区的全部命令。

use crate::ids::{ContainerRequestId, TabId};
use crate::model::container::{
    ComposeAction, ComposeProject, ContainerBuildSpec, ContainerConnection, ContainerCreateSpec,
    ContainerEngineProfile, ContainerLifecycleAction, ContainerNetworkSpec, ContainerRegistry,
    ContainerRegistryAuth, ContainerUpdateSpec,
};
use std::path::PathBuf;

/// 容器管理工作区命令。
///
/// 工作区以 [`TabId`] 作为引擎会话句柄：`Connect` 成功后，后续查询都带
/// 同一个 `tab_id`；[`ContainerRequestId`] 只用于关联单次请求的多条事件。
pub enum ContainerCommand {
    /// 打开引擎工作区：解析并探测连接配置。
    Connect {
        tab_id: TabId,
        profile: ContainerEngineProfile,
    },
    /// 关闭工作区：中止订阅与 exec 会话，不停止容器。
    Disconnect { tab_id: TabId },
    /// 容器表单的“测试连接”：只探测，不建立工作区。
    TestConnection {
        request_id: ContainerRequestId,
        profile: ContainerEngineProfile,
    },
    /// 编排项目命令（up/down）；只能走宿主机 CLI。
    RunCompose {
        tab_id: TabId,
        request_id: ContainerRequestId,
        project: Box<ComposeProject>,
        action: ComposeAction,
    },
    /// 发现本机引擎：让用户不必手填 socket/管道路径就能添加连接。
    Discover { request_id: ContainerRequestId },
    ListContainers {
        tab_id: TabId,
        request_id: ContainerRequestId,
    },
    InspectContainer {
        tab_id: TabId,
        request_id: ContainerRequestId,
        container: String,
    },
    /// 容器启停删。
    Lifecycle {
        tab_id: TabId,
        request_id: ContainerRequestId,
        container: String,
        action: ContainerLifecycleAction,
    },
    /// 动态更新容器配置（CPU/内存限制与重启策略）。
    UpdateContainer {
        tab_id: TabId,
        request_id: ContainerRequestId,
        container: String,
        spec: ContainerUpdateSpec,
    },
    /// 将容器当前状态就地提交为新镜像（Commit）。
    CommitContainer {
        tab_id: TabId,
        request_id: ContainerRequestId,
        container: String,
        repo: String,
        tag: Option<String>,
        comment: Option<String>,
        author: Option<String>,
    },
    /// 检查容器文件系统变更（Diff）。
    DiffContainer {
        tab_id: TabId,
        request_id: ContainerRequestId,
        container: String,
    },
    /// 在容器内打开交互式终端；容器与执行参数都在 `options` 里。
    ExecTerminal {
        tab_id: TabId,
        request_id: ContainerRequestId,
        options: ContainerConnection,
        size: crate::transport::TerminalSize,
    },
    ListImages {
        tab_id: TabId,
        request_id: ContainerRequestId,
    },
    InspectImage {
        tab_id: TabId,
        request_id: ContainerRequestId,
        image: String,
    },
    PullImage {
        tab_id: TabId,
        request_id: ContainerRequestId,
        /// `repo:tag` 或镜像摘要。
        reference: String,
    },
    RemoveImage {
        tab_id: TabId,
        request_id: ContainerRequestId,
        image: String,
        force: bool,
    },
    TagImage {
        tab_id: TabId,
        request_id: ContainerRequestId,
        image: String,
        repo: String,
        tag: Option<String>,
    },
    PushImage {
        tab_id: TabId,
        request_id: ContainerRequestId,
        image: String,
        tag: Option<String>,
        auth: Option<ContainerRegistryAuth>,
    },
    /// 先给本地镜像打目标标签，再把目标引用推送到远端仓库。
    PushTaggedImage {
        tab_id: TabId,
        request_id: ContainerRequestId,
        source_image: String,
        repo: String,
        tag: Option<String>,
        auth: Option<ContainerRegistryAuth>,
    },
    BuildImage {
        tab_id: TabId,
        request_id: ContainerRequestId,
        spec: ContainerBuildSpec,
    },
    /// 导出镜像为本地 Tar 归档文件（Save）。
    SaveImage {
        tab_id: TabId,
        request_id: ContainerRequestId,
        image: String,
        local_path: std::path::PathBuf,
    },
    /// 从本地 Tar 归档文件导入镜像（Load）。
    LoadImage {
        tab_id: TabId,
        request_id: ContainerRequestId,
        local_path: std::path::PathBuf,
    },
    /// 备份数据卷为本地 Tar 归档文件。
    BackupVolume {
        tab_id: TabId,
        request_id: ContainerRequestId,
        volume: String,
        local_path: std::path::PathBuf,
    },
    /// 从本地 Tar 归档文件恢复到数据卷中。
    RestoreVolume {
        tab_id: TabId,
        request_id: ContainerRequestId,
        volume: String,
        local_path: std::path::PathBuf,
    },
    /// 在指定镜像仓库搜索镜像（Docker Hub 或私有 Registry）。
    SearchImages {
        tab_id: TabId,
        request_id: ContainerRequestId,
        registry: ContainerRegistry,
        term: String,
        limit: Option<u32>,
    },
    /// 读取指定仓库的实时标签列表。
    ListImageTags {
        tab_id: TabId,
        request_id: ContainerRequestId,
        registry: ContainerRegistry,
        repository: String,
    },
    /// 带任务追踪与分层进度的镜像拉取指令。
    PullImageTask {
        tab_id: TabId,
        task_id: String,
        reference: String,
        auth: Option<ContainerRegistryAuth>,
    },
    /// 取消正在进行的镜像拉取任务。
    CancelPullTask { tab_id: TabId, task_id: String },
    ListVolumes {
        tab_id: TabId,
        request_id: ContainerRequestId,
    },
    InspectVolume {
        tab_id: TabId,
        request_id: ContainerRequestId,
        name: String,
    },
    CreateVolume {
        tab_id: TabId,
        request_id: ContainerRequestId,
        name: String,
        driver: Option<String>,
    },
    RemoveVolume {
        tab_id: TabId,
        request_id: ContainerRequestId,
        name: String,
        force: bool,
    },
    ListNetworks {
        tab_id: TabId,
        request_id: ContainerRequestId,
    },
    InspectNetwork {
        tab_id: TabId,
        request_id: ContainerRequestId,
        network: String,
    },
    CreateNetwork {
        tab_id: TabId,
        request_id: ContainerRequestId,
        spec: ContainerNetworkSpec,
    },
    RemoveNetwork {
        tab_id: TabId,
        request_id: ContainerRequestId,
        network: String,
    },
    ConnectNetwork {
        tab_id: TabId,
        request_id: ContainerRequestId,
        network: String,
        container: String,
    },
    DisconnectNetwork {
        tab_id: TabId,
        request_id: ContainerRequestId,
        network: String,
        container: String,
        force: bool,
    },
    /// Compose 项目列表（按容器标签聚合）。
    ListComposeProjects {
        tab_id: TabId,
        request_id: ContainerRequestId,
    },
    /// 读取容器日志；`follow` 为真时持续推送到取消或容器结束。
    Logs {
        tab_id: TabId,
        request_id: ContainerRequestId,
        container: String,
        tail: usize,
        follow: bool,
        /// 是否让引擎附带服务端时间戳（`tail=0` 表示全量）。
        timestamps: bool,
    },
    /// 读取编排项目多容器联合日志。
    ProjectLogs {
        tab_id: TabId,
        request_id: ContainerRequestId,
        project: String,
        containers: Vec<String>,
        tail: usize,
        follow: bool,
        timestamps: bool,
    },
    /// 开启/关闭统计轮询；同一容器重复开启会替换旧订阅。
    Stats {
        tab_id: TabId,
        request_id: ContainerRequestId,
        container: String,
        enabled: bool,
        /// 轮询间隔（毫秒）：界面提供 1s/3s/10s 三档，0 按 3s 处理。
        interval_ms: u64,
    },
    /// 列出 Podman Quadlet 单元（宿主 systemd 用户单元）。
    ListQuadlets {
        tab_id: TabId,
        request_id: ContainerRequestId,
    },
    /// 对 Quadlet 单元执行启停/重启。
    QuadletAction {
        tab_id: TabId,
        request_id: ContainerRequestId,
        unit: String,
        action: crate::model::container::ContainerQuadletAction,
    },
    /// 列出 Podman Pod。
    ListPods {
        tab_id: TabId,
        request_id: ContainerRequestId,
    },
    /// 对 Podman Pod 执行启停/重启/删除。
    PodAction {
        tab_id: TabId,
        request_id: ContainerRequestId,
        pod: String,
        action: crate::model::container::ContainerPodAction,
    },
    /// 容器内进程列表：进程数与 CPU/内存占用，用于统计页的进程表。
    ListProcesses {
        tab_id: TabId,
        request_id: ContainerRequestId,
        container: String,
    },
    /// 取消进行中的日志跟随、上传或下载。
    Cancel {
        tab_id: TabId,
        request_id: ContainerRequestId,
    },
    ListDir {
        tab_id: TabId,
        request_id: ContainerRequestId,
        container: String,
        /// 容器内绝对路径。
        path: String,
    },
    /// 列出数据卷内的文件与目录。
    ListVolumeDir {
        tab_id: TabId,
        request_id: ContainerRequestId,
        volume: String,
        path: String,
    },
    ReadFile {
        tab_id: TabId,
        request_id: ContainerRequestId,
        container: String,
        path: String,
        /// 预览上限；超出部分截断并标记。
        max_bytes: usize,
    },
    /// 读取数据卷内文件文本预览。
    ReadVolumeFile {
        tab_id: TabId,
        request_id: ContainerRequestId,
        volume: String,
        path: String,
        max_bytes: usize,
    },
    Download {
        tab_id: TabId,
        request_id: ContainerRequestId,
        container: String,
        path: String,
        local_path: PathBuf,
    },
    /// 从数据卷下载文件到本地。
    DownloadVolumeFile {
        tab_id: TabId,
        request_id: ContainerRequestId,
        volume: String,
        path: String,
        local_path: PathBuf,
    },
    /// 把容器内目录原样导出为本地 tar 归档。
    DownloadDirectory {
        tab_id: TabId,
        request_id: ContainerRequestId,
        container: String,
        path: String,
        local_path: PathBuf,
    },
    Upload {
        tab_id: TabId,
        request_id: ContainerRequestId,
        container: String,
        /// 容器内目标路径（含文件名）。
        path: String,
        local_path: PathBuf,
    },
    /// 上传本地目录到容器内目标目录，保留目录层级。
    UploadDirectory {
        tab_id: TabId,
        request_id: ContainerRequestId,
        container: String,
        path: String,
        local_path: PathBuf,
    },
    RemovePath {
        tab_id: TabId,
        request_id: ContainerRequestId,
        container: String,
        path: String,
        recursive: bool,
    },
    /// 复制容器内文件或目录到目标路径。
    CopyPath {
        tab_id: TabId,
        request_id: ContainerRequestId,
        container: String,
        from: String,
        to: String,
    },
    /// 修改容器内路径权限。
    ChmodPath {
        tab_id: TabId,
        request_id: ContainerRequestId,
        container: String,
        path: String,
        mode: String,
    },
    /// 修改容器内路径属主。
    ChownPath {
        tab_id: TabId,
        request_id: ContainerRequestId,
        container: String,
        path: String,
        owner: String,
    },
    /// 就地保存/写入容器内文本文件。
    SaveFile {
        tab_id: TabId,
        request_id: ContainerRequestId,
        container: String,
        path: String,
        content: String,
    },
    /// 就地保存/写入数据卷内文本文件。
    SaveVolumeFile {
        tab_id: TabId,
        request_id: ContainerRequestId,
        volume: String,
        path: String,
        content: String,
    },
    Rename {
        tab_id: TabId,
        request_id: ContainerRequestId,
        container: String,
        from: String,
        to: String,
    },
    /// 重命名容器。
    RenameContainer {
        tab_id: TabId,
        request_id: ContainerRequestId,
        container: String,
        name: String,
    },
    /// 创建并启动容器（把镜像跑起来）。
    CreateContainer {
        tab_id: TabId,
        request_id: ContainerRequestId,
        spec: ContainerCreateSpec,
    },
    /// 就地重建容器：停止并移除旧容器后，以修改后的新配置创建并启动新容器。
    RecreateContainer {
        tab_id: TabId,
        request_id: ContainerRequestId,
        old_container_id: String,
        spec: ContainerCreateSpec,
    },
    /// 在容器内创建目录：归档接口只能写入已存在的目录，上传前先建目录才可用。
    CreateDir {
        tab_id: TabId,
        request_id: ContainerRequestId,
        container: String,
        path: String,
    },
    /// 清理未被任何容器使用的数据卷。
    PruneVolumes {
        tab_id: TabId,
        request_id: ContainerRequestId,
    },
    /// 清理悬空镜像。
    PruneImages {
        tab_id: TabId,
        request_id: ContainerRequestId,
    },
    /// 清理未使用的容器网络。
    PruneNetworks {
        tab_id: TabId,
        request_id: ContainerRequestId,
    },
    /// 查询全局系统磁盘占用（对应 Docker system df）。
    SystemDf {
        tab_id: TabId,
        request_id: ContainerRequestId,
    },
    /// 执行系统级联合清理（对应 Docker system prune）。
    SystemPrune {
        tab_id: TabId,
        request_id: ContainerRequestId,
        all: bool,
        volumes: bool,
    },
}
