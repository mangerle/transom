//! 应用分领域命令枚举定义。
//!
//! 将全局单一的 AppCommand 按照业务领域归类为分领域子命令：
//! - TerminalCommand: Connect, CloseTab, Shutdown
//! - SftpCommand: Sftp (文件操作), SftpCancel (取消任务)
//! - ForwardCommand: ForwardStart, ForwardStop
//! - HostCommand: DeployPublicKey, RunHostActions, SampleProcesses, SampleProcessCount, KillProcess
//! - SyncCommand: CloudSyncPush, CloudSyncPull, GitSync
//! - ZmodemCommand: ZmodemUpload, ZmodemCancel

use crate::ids::{ConnectionId, ContainerRequestId, ForwardId, TabId, TransferId};
use crate::model::container::{
    ContainerConnection, ContainerEngineProfile, ContainerLifecycleAction,
};
use crate::model::forward::ForwardRule;
use crate::model::host::HostAction;
use crate::model::session::Session;
use crate::model::settings::SyncSettings;
use crate::protocol::sftp::SftpOperation;
use crate::secrets::ConnectionSecrets;
use crate::transport::TerminalSize;
use std::path::PathBuf;

/// 终端建连参数载荷，收敛多参数并消除枚举变体尺寸倾斜。
pub struct ConnectParams {
    pub tab_id: TabId,
    pub connection_id: ConnectionId,
    pub session: Session,
    pub jumps: Vec<crate::model::session::SshHop>,
    pub proxy: crate::protocol::proxy::OutboundProxy,
    pub size: TerminalSize,
    pub secrets: ConnectionSecrets,
    pub zmodem: crate::protocol::zmodem::ZmodemSettings,
    pub allow_sha1_publickey_auth: bool,
    /// 是否启用 Shell 集成（提示符导航与退出码识别）。
    ///
    /// 随命令携带而不是让控制器回读设置：控制器不持有设置文件句柄，
    /// 与 zmodem 配置是同一套约定。本地会话据此决定要不要用启动参数预装集成脚本。
    pub shell_integration: bool,
}

/// 终端建连与生命周期命令。
pub enum TerminalCommand {
    Connect(Box<ConnectParams>),
    CloseTab { tab_id: TabId },
    Shutdown,
}

/// SFTP 远程文件操作命令。
pub enum SftpCommand {
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
}

/// 端口转发命令。
pub enum ForwardCommand {
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
}

/// 主机监控与运维命令。
pub enum HostCommand {
    DeployPublicKey {
        tab_id: TabId,
        connection_id: ConnectionId,
        key_name: String,
        public_key: String,
    },
    RunHostActions {
        actions: Vec<HostAction>,
    },
    SampleProcesses {
        tab_id: TabId,
        connection_id: ConnectionId,
    },
    /// 只采样进程总数：进程卡片折叠时用它维持摘要计数，避免拉取全量明细。
    SampleProcessCount {
        tab_id: TabId,
        connection_id: ConnectionId,
    },
    KillProcess {
        tab_id: TabId,
        connection_id: ConnectionId,
        pid: u32,
        force: bool,
    },
}

/// 云备份与 Git 同步命令。
pub enum SyncCommand {
    CloudSyncPush {
        settings: SyncSettings,
        catalog: crate::model::catalog::SessionCatalog,
    },
    CloudSyncPull {
        settings: SyncSettings,
    },
    GitSync {
        settings: SyncSettings,
        catalog: crate::model::catalog::SessionCatalog,
    },
}

/// ZMODEM 传输命令。
pub enum ZmodemCommand {
    ZmodemUpload { tab_id: TabId, files: Vec<PathBuf> },
    ZmodemCancel { tab_id: TabId },
}

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
        name: String,
        driver: Option<String>,
    },
    RemoveNetwork {
        tab_id: TabId,
        request_id: ContainerRequestId,
        network: String,
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
    /// 开启/关闭统计轮询；同一容器重复开启会替换旧订阅。
    Stats {
        tab_id: TabId,
        request_id: ContainerRequestId,
        container: String,
        enabled: bool,
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
    ReadFile {
        tab_id: TabId,
        request_id: ContainerRequestId,
        container: String,
        path: String,
        /// 预览上限；超出部分截断并标记。
        max_bytes: usize,
    },
    Download {
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
    RemovePath {
        tab_id: TabId,
        request_id: ContainerRequestId,
        container: String,
        path: String,
        recursive: bool,
    },
    Rename {
        tab_id: TabId,
        request_id: ContainerRequestId,
        container: String,
        from: String,
        to: String,
    },
    /// 在容器内创建目录：归档接口只能写入已存在的目录，上传前先建目录才可用。
    CreateDir {
        tab_id: TabId,
        request_id: ContainerRequestId,
        container: String,
        path: String,
    },
}
