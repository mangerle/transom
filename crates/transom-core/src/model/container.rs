//! 容器运行时（Docker / Podman）的连接参数与管理面数据类型。
//!
//! 设计原理（Why）：
//! - 引擎端点被建模为「引用」而不是一个裸字符串：`Auto` 让同一份会话目录
//!   可以在不同机器上工作（Linux 用 unix socket、Windows 用命名管道），
//!   显式端点则用于多引擎并存或远端守护进程；
//! - 管理面只暴露 UI 需要的字段，API 原始 JSON 在 `transom-container` 内部
//!   反序列化后立即收敛为本模块的强类型，避免界面层跟着上游字段漂移；
//! - 所有枚举都保留 `Unknown`/`Other` 兜底：外部引擎的版本差异不应该让
//!   整个会话目录或列表反序列化失败。

use crate::ids::EngineProfileId;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;

/// 容器引擎种类。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ContainerEngineKind {
    /// Docker Engine 或与 Docker 兼容的发行版。
    #[default]
    Docker,
    /// Podman（走 Docker 兼容 REST 路径）。
    Podman,
}

impl ContainerEngineKind {
    /// 界面与日志使用的稳定名称。
    pub fn label(self) -> &'static str {
        match self {
            Self::Docker => "Docker",
            Self::Podman => "Podman",
        }
    }

    /// 本机 CLI 兜底时优先尝试的可执行文件名。
    pub fn cli_program(self) -> &'static str {
        match self {
            Self::Docker => "docker",
            Self::Podman => "podman",
        }
    }

    /// 端点语法前缀（`unix://`、`npipe://`、`ssh://` 等）。
    pub fn scheme(self) -> &'static str {
        match self {
            Self::Docker => "docker",
            Self::Podman => "podman",
        }
    }
}

impl fmt::Display for ContainerEngineKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// 可保存的引擎连接配置（容器表单的产物）。
///
/// 设计原理（Why）：连接配置与终端会话分开建模——一个引擎连接打开的是
/// 管理工作区，而不是一条终端会话；`updated_at` 让备份/云同步沿用会话的
/// 时间戳裁决规则。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerEngineProfile {
    pub id: EngineProfileId,
    pub name: String,
    #[serde(default)]
    pub engine: ContainerEngineRef,
    /// 最后修改时间（Unix 毫秒），供多端合并判定谁更新。
    #[serde(default)]
    pub updated_at: u64,
}

impl ContainerEngineProfile {
    pub fn new(name: impl Into<String>, engine: ContainerEngineRef) -> Self {
        Self {
            id: EngineProfileId::new(),
            name: name.into(),
            engine,
            updated_at: now_millis(),
        }
    }

    pub fn touch(&mut self) {
        self.updated_at = now_millis();
    }

    /// 界面标题：优先用户命名，空名回落到引擎摘要。
    pub fn display_name(&self) -> String {
        let name = self.name.trim();
        if name.is_empty() {
            self.engine.describe()
        } else {
            name.to_string()
        }
    }
}

/// 当前 Unix 毫秒时间戳；时钟异常时按 0 处理。
fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|duration| u64::try_from(duration.as_millis()).ok())
        .unwrap_or(0)
}

/// 引擎接入端点。
///
/// 序列化后随会话落盘，因此字段必须保持向后兼容：新增变体时老版本会因
/// 未知 `kind` 拒绝整份目录，升级前需要评估迁移策略。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ContainerEndpoint {
    /// 由本机按平台与引擎种类自动发现。
    #[default]
    Auto,
    /// Unix 域套接字（Linux/macOS）。
    Unix { path: PathBuf },
    /// Windows 命名管道，如 `//./pipe/docker_engine`。
    NamedPipe { path: String },
    /// TCP 端点（如 `tcp://127.0.0.1:2375`）；仅允许显式配置。
    Tcp { url: String },
    /// 远端守护进程，通过 `docker system dial-stdio` 建立原始流。
    Ssh { target: String },
    /// WSL 发行版内的 CLI 兜底。
    Wsl { distro: String },
    /// WSLC（WSL containers）CLI 兜底。
    Wslc,
    /// 任意 CLI 兜底：`program` 加固定前缀参数。
    Cli { program: String, args: Vec<String> },
}

impl ContainerEndpoint {
    /// 日志与界面摘要；不含任何凭证。
    pub fn describe(&self) -> String {
        match self {
            Self::Auto => "自动发现".to_string(),
            Self::Unix { path } => format!("unix://{}", path.display()),
            Self::NamedPipe { path } => format!("npipe:{}", display_pipe_path(path)),
            Self::Tcp { url } => url.clone(),
            Self::Ssh { target } => format!("ssh://{target}"),
            Self::Wsl { distro } => format!("wsl://{distro}"),
            Self::Wslc => "wslc://default".to_string(),
            Self::Cli { program, .. } => format!("cli://{program}"),
        }
    }

    /// 是否为 CLI 后端：能力与 resize 支持都和 HTTP 后端不同。
    pub fn is_cli(&self) -> bool {
        matches!(self, Self::Wsl { .. } | Self::Wslc | Self::Cli { .. })
    }

    pub fn is_auto(&self) -> bool {
        matches!(self, Self::Auto)
    }

    /// 当前平台是否支持该端点；用于表单在保存前给出明确提示。
    ///
    /// # Errors
    /// 端点与宿主平台不匹配（如在 Linux 上选择命名管道）时返回中文原因。
    pub fn ensure_supported(&self) -> Result<(), String> {
        match self {
            Self::Ssh { .. } => Err("SSH 引擎端点将在后续版本提供".to_string()),
            Self::NamedPipe { .. } if !cfg!(windows) => Err("命名管道仅支持 Windows".to_string()),
            Self::Wsl { .. } | Self::Wslc if !cfg!(windows) => {
                Err("WSL/WSLC 端点仅支持 Windows".to_string())
            }
            _ => Ok(()),
        }
    }
}

/// 去掉 Windows 命名管道前缀里的反斜杠转义，只用于展示。
fn display_pipe_path(path: &str) -> String {
    path.replace('\\', "/")
}

/// 会话与命令携带的引擎引用。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ContainerEngineRef {
    pub kind: ContainerEngineKind,
    #[serde(default)]
    pub endpoint: ContainerEndpoint,
}

impl ContainerEngineRef {
    pub fn auto(kind: ContainerEngineKind) -> Self {
        Self {
            kind,
            endpoint: ContainerEndpoint::Auto,
        }
    }

    pub fn explicit(kind: ContainerEngineKind, endpoint: ContainerEndpoint) -> Self {
        Self { kind, endpoint }
    }

    /// 界面摘要：`Docker（自动发现）`、`Podman（unix:///run/...）`。
    pub fn describe(&self) -> String {
        format!("{}（{}）", self.kind.label(), self.endpoint.describe())
    }
}

/// 容器 exec 参数：在容器内打开交互式终端。
///
/// 工作区已经持有引擎会话，因此这里不再重复携带引擎端点；它只是一份
/// “在哪个容器、以什么身份、执行什么命令”的描述。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerConnection {
    /// 容器名或 ID；名称在运行期解析，重命名后由用户修改会话。
    pub container: String,
    /// 容器内执行用户；`None` 表示镜像默认用户。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    /// 容器内工作目录；`None` 表示镜像默认工作目录。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
    /// 启动命令；`None` 表示引擎默认（通常是 `/bin/sh`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<Vec<String>>,
    /// 追加环境变量（`KEY=VALUE`），原样传给 exec，不经过 Shell。
    #[serde(default)]
    pub env: Vec<String>,
    /// 是否申请 TTY；交互式终端固定为 `true`。
    #[serde(default = "default_true")]
    pub tty: bool,
}

fn default_true() -> bool {
    true
}

impl Default for ContainerConnection {
    fn default() -> Self {
        Self {
            container: String::new(),
            user: None,
            working_dir: None,
            command: None,
            env: Vec::new(),
            tty: true,
        }
    }
}

impl ContainerConnection {
    pub fn new(container: impl Into<String>) -> Self {
        Self {
            container: container.into(),
            ..Self::default()
        }
    }

    /// 实际执行的 argv；未配置时回落到镜像的默认 Shell。
    pub fn command_argv(&self) -> Vec<String> {
        self.command
            .clone()
            .filter(|argv| !argv.is_empty())
            .unwrap_or_else(|| vec!["/bin/sh".to_string()])
    }
}

/// 已发现的引擎实例。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerEngineInfo {
    /// 已解析为显式端点的引用，UI 原样带回后续命令。
    pub engine: ContainerEngineRef,
    /// 服务端版本，如 `27.3.1`。
    pub version: String,
    /// 协商后的 API 版本，如 `1.46`。
    pub api_version: String,
    /// 服务端平台名，如 `Docker Engine - Community` / `Podman Engine`。
    pub platform: String,
    /// CLI 后端不具备 resize 与部分流式能力。
    pub cli_backend: bool,
}

impl ContainerEngineInfo {
    /// 界面标题：`Docker 27.3.1`。
    pub fn title(&self) -> String {
        format!("{} {}", self.engine.kind.label(), self.version)
    }
}

/// 容器运行状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ContainerState {
    Created,
    Running,
    Paused,
    Restarting,
    Removing,
    Exited,
    Dead,
    #[default]
    Unknown,
}

impl ContainerState {
    /// 从引擎返回的字符串解析；未知值一律降级为 `Unknown`。
    pub fn from_api(value: &str) -> Self {
        match value {
            "created" => Self::Created,
            "running" => Self::Running,
            "paused" => Self::Paused,
            "restarting" => Self::Restarting,
            "removing" => Self::Removing,
            "exited" => Self::Exited,
            "dead" => Self::Dead,
            _ => Self::Unknown,
        }
    }

    pub fn is_running(self) -> bool {
        matches!(self, Self::Running | Self::Paused | Self::Restarting)
    }

    /// i18n 键后缀；界面统一用 `container.state.*` 渲染。
    pub fn key(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Running => "running",
            Self::Paused => "paused",
            Self::Restarting => "restarting",
            Self::Removing => "removing",
            Self::Exited => "exited",
            Self::Dead => "dead",
            Self::Unknown => "unknown",
        }
    }
}

/// 端口映射。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerPort {
    pub private_port: u16,
    #[serde(default)]
    pub public_port: Option<u16>,
    #[serde(default)]
    pub ip: Option<String>,
    #[serde(default)]
    pub protocol: String,
}

/// 容器列表项。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerSummary {
    pub id: String,
    pub name: String,
    pub image: String,
    pub state: ContainerState,
    /// 引擎给出的可读状态，如 `Up 3 hours`。
    pub status: String,
    /// 创建时间（Unix 秒）。
    pub created: i64,
    #[serde(default)]
    pub ports: Vec<ContainerPort>,
    /// Compose 项目名（来自容器标签）；非 Compose 容器为 `None`。
    #[serde(default)]
    pub compose_project: Option<String>,
    /// Compose 项目的工作目录（来自容器标签）。
    #[serde(default)]
    pub compose_working_dir: Option<String>,
    /// Compose 项目的配置文件列表（来自容器标签）。
    #[serde(default)]
    pub compose_config_files: Vec<String>,
}

impl ContainerSummary {
    /// 端口摘要：`8080->80/tcp`，无映射时为空串。
    pub fn ports_label(&self) -> String {
        let mut parts = Vec::with_capacity(self.ports.len());
        for port in &self.ports {
            let protocol = if port.protocol.is_empty() {
                "tcp"
            } else {
                port.protocol.as_str()
            };
            match port.public_port {
                Some(public) => parts.push(format!("{public}->{}/{protocol}", port.private_port)),
                None => parts.push(format!("{}/{protocol}", port.private_port)),
            }
        }
        parts.join(", ")
    }
}

/// 镜像列表项。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerImageSummary {
    pub id: String,
    /// `Repo:Tag` 列表；悬空镜像可能为空。
    #[serde(default)]
    pub tags: Vec<String>,
    /// 字节数。
    pub size: u64,
    pub created: i64,
}

impl ContainerImageSummary {
    /// 界面主标题：优先仓库标签，悬空镜像回落到短 ID。
    pub fn display_name(&self) -> String {
        self.tags
            .first()
            .cloned()
            .unwrap_or_else(|| short_id(&self.id))
    }
}

/// 容器挂载。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ContainerMount {
    /// `bind` / `volume` / `tmpfs` 等。
    pub kind: String,
    /// 宿主侧来源（卷名或宿主路径）。
    pub source: String,
    /// 容器内目标路径。
    pub destination: String,
    #[serde(default)]
    pub mode: String,
    #[serde(default)]
    pub rw: bool,
}

/// 容器详情（基础信息页签）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContainerDetails {
    pub id: String,
    pub name: String,
    pub image: String,
    pub image_id: String,
    pub command: String,
    #[serde(default)]
    pub entrypoint: Vec<String>,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub working_dir: Option<String>,
    #[serde(default)]
    pub env: Vec<String>,
    pub state: ContainerState,
    pub status: String,
    pub created: i64,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub finished_at: Option<String>,
    #[serde(default)]
    pub exit_code: Option<i64>,
    #[serde(default)]
    pub restart_policy: Option<String>,
    #[serde(default)]
    pub ports: Vec<ContainerPort>,
    #[serde(default)]
    pub mounts: Vec<ContainerMount>,
    #[serde(default)]
    pub compose_project: Option<String>,
    /// 原始 inspect JSON（格式化后），用于“高级信息”折叠区。
    #[serde(default)]
    pub inspect: String,
}

/// 容器资源统计快照。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ContainerStats {
    /// CPU 占用百分比（0-100，按全部核心归一）。
    pub cpu_percent: f32,
    pub memory_used: u64,
    pub memory_limit: u64,
    /// 内存占用百分比（0-100）。
    pub memory_percent: f32,
    pub network_rx: u64,
    pub network_tx: u64,
    pub block_read: u64,
    pub block_write: u64,
    /// 进程/线程数。
    pub pids: u64,
}

/// 镜像详情（基础信息页签）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerImageDetails {
    pub id: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub size: u64,
    pub created: i64,
    #[serde(default)]
    pub os: Option<String>,
    #[serde(default)]
    pub architecture: Option<String>,
    /// 层数量；引擎响应缺失时为 0。
    #[serde(default)]
    pub layers: usize,
}

/// 数据卷摘要。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ContainerVolumeSummary {
    pub name: String,
    pub driver: String,
    pub mountpoint: String,
    #[serde(default)]
    pub created: Option<String>,
    #[serde(default)]
    pub labels: Vec<(String, String)>,
}

/// 数据卷详情。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ContainerVolumeDetails {
    pub summary: ContainerVolumeSummary,
    /// 正在使用该卷的容器名。
    #[serde(default)]
    pub used_by: Vec<String>,
}

/// 网络摘要。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ContainerNetworkSummary {
    pub id: String,
    pub name: String,
    pub driver: String,
    pub scope: String,
    #[serde(default)]
    pub subnet: Option<String>,
}

/// 网络上的容器连接。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ContainerNetworkAttachment {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub ipv4: Option<String>,
}

/// 网络详情。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ContainerNetworkDetails {
    pub summary: ContainerNetworkSummary,
    #[serde(default)]
    pub gateway: Option<String>,
    #[serde(default)]
    pub connected: Vec<ContainerNetworkAttachment>,
}

/// Compose 项目（按容器标签聚合）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ComposeProject {
    pub name: String,
    pub running: usize,
    pub total: usize,
    #[serde(default)]
    pub working_dir: Option<String>,
    #[serde(default)]
    pub config_files: Vec<String>,
}

impl ComposeProject {
    /// 项目状态摘要：`3/5 运行中`。
    pub fn state_label(&self) -> String {
        format!("{}/{} 运行中", self.running, self.total)
    }
}

/// 按容器标签聚合 Compose 项目。
///
/// 设计原理（Why）：Docker Engine 没有 Compose 专有 API，项目信息只存在于
/// 容器标签中；聚合放在领域层，HTTP 与 CLI 后端共用一套口径。
/// 代价：如果项目内所有容器都被删除，项目本身就不再可见。
pub fn compose_projects(containers: &[ContainerSummary]) -> Vec<ComposeProject> {
    let mut projects: std::collections::BTreeMap<String, ComposeProject> =
        std::collections::BTreeMap::new();
    for container in containers {
        let Some(name) = container.compose_project.as_deref() else {
            continue;
        };
        let project = projects
            .entry(name.to_string())
            .or_insert_with(|| ComposeProject {
                name: name.to_string(),
                running: 0,
                total: 0,
                working_dir: container.compose_working_dir.clone(),
                config_files: container.compose_config_files.clone(),
            });
        project.total += 1;
        if container.state == ContainerState::Running {
            project.running += 1;
        }
        if project.working_dir.is_none() {
            project.working_dir = container.compose_working_dir.clone();
        }
        if project.config_files.is_empty() {
            project.config_files = container.compose_config_files.clone();
        }
    }
    projects.into_values().collect()
}

/// 容器内文件类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ContainerFileKind {
    Directory,
    File,
    Symlink,
    #[default]
    Other,
}

/// 容器内文件条目。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerFileEntry {
    pub name: String,
    /// 容器内绝对路径。
    pub path: String,
    pub kind: ContainerFileKind,
    pub size: u64,
    /// Unix 权限位（如 `0o644`）；归档 API 只提供 tar 头里的模式。
    pub mode: u32,
    /// 修改时间（Unix 秒）。
    #[serde(default)]
    pub modified: Option<i64>,
    #[serde(default)]
    pub link_target: Option<String>,
}

/// 容器生命周期动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum ContainerLifecycleAction {
    Start,
    Stop,
    Restart,
    Remove {
        #[serde(default)]
        force: bool,
    },
}

impl ContainerLifecycleAction {
    /// 日志与 UI 使用的动作名。
    pub fn label(self) -> &'static str {
        match self {
            Self::Start => "启动",
            Self::Stop => "停止",
            Self::Restart => "重启",
            Self::Remove { .. } => "删除",
        }
    }
}

/// 截短容器/镜像 ID，供列表与日志定位。
pub fn short_id(id: &str) -> String {
    let trimmed = id.trim_start_matches("sha256:");
    trimmed.chars().take(12).collect()
}

#[cfg(test)]
mod tests {
    use super::{
        ContainerEndpoint, ContainerEngineKind, ContainerEngineRef, ContainerState,
        ContainerSummary, compose_projects, short_id,
    };

    #[test]
    fn endpoint_round_trips_through_json() {
        let endpoint = ContainerEndpoint::Unix {
            path: "/var/run/docker.sock".into(),
        };
        let json = serde_json::to_string(&endpoint).expect("序列化必须成功");
        let decoded: ContainerEndpoint = serde_json::from_str(&json).expect("反序列化必须成功");
        assert_eq!(decoded, endpoint);

        let auto = serde_json::to_string(&ContainerEndpoint::Auto).expect("序列化必须成功");
        let decoded: ContainerEndpoint = serde_json::from_str(&auto).expect("反序列化必须成功");
        assert!(decoded.is_auto());
    }

    #[test]
    fn engine_ref_defaults_to_docker_auto() {
        let reference = ContainerEngineRef::default();
        assert_eq!(reference.kind, ContainerEngineKind::Docker);
        assert!(reference.endpoint.is_auto());
    }

    #[test]
    fn unknown_state_degrades_instead_of_failing() {
        assert_eq!(
            ContainerState::from_api("hibernating"),
            ContainerState::Unknown
        );
        assert!(ContainerState::from_api("running").is_running());
    }

    #[test]
    fn short_id_strips_algorithm_prefix() {
        assert_eq!(short_id("sha256:0123456789abcdef"), "0123456789ab");
        assert_eq!(short_id("abc"), "abc");
    }

    #[test]
    fn cli_endpoints_are_flagged() {
        assert!(ContainerEndpoint::Wslc.is_cli());
        assert!(
            ContainerEndpoint::Cli {
                program: "docker".into(),
                args: Vec::new()
            }
            .is_cli()
        );
        assert!(!ContainerEndpoint::Auto.is_cli());
    }

    #[test]
    fn compose_projects_group_containers_by_label() {
        let mut running = summary("web-1", ContainerState::Running);
        running.compose_project = Some("shop".into());
        running.compose_working_dir = Some("/srv/shop".into());
        running.compose_config_files = vec!["/srv/shop/compose.yml".into()];
        let mut stopped = summary("web-2", ContainerState::Exited);
        stopped.compose_project = Some("shop".into());
        let mut other = summary("standalone", ContainerState::Running);
        other.compose_project = None;

        let projects = compose_projects(&[running, stopped, other]);
        assert_eq!(projects.len(), 1, "非 Compose 容器不参与聚合");
        let project = projects.first().expect("项目存在");
        assert_eq!(project.name, "shop");
        assert_eq!(project.running, 1);
        assert_eq!(project.total, 2);
        assert_eq!(project.state_label(), "1/2 运行中");
        assert_eq!(project.working_dir.as_deref(), Some("/srv/shop"));
        assert_eq!(project.config_files.len(), 1);
    }

    fn summary(name: &str, state: ContainerState) -> ContainerSummary {
        ContainerSummary {
            id: format!("id-{name}"),
            name: name.to_string(),
            image: "demo".to_string(),
            state,
            status: String::new(),
            created: 0,
            ports: Vec::new(),
            compose_project: None,
            compose_working_dir: None,
            compose_config_files: Vec::new(),
        }
    }
}
