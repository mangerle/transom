//! 管理面条目：容器、镜像、卷、网络、编排与容器内文件。

use super::short_id;
use serde::{Deserialize, Serialize};

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
    /// 容器的挂载列表；列表接口就返回，用于「哪些卷正被使用」这类筛选，
    /// 不必对每个容器再发一次 inspect。
    #[serde(default)]
    pub mounts: Vec<ContainerMount>,
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

    /// 列表用的短标题：剥掉仓库主机与官方 `library/` 前缀，只留 `名字[:标签]`。
    ///
    /// 设计原理（Why）：走国内加速仓库时完整引用动辄三四十字符
    /// （`docker.m.daocloud.io/library/busybox:1.36`），在任何宽度的列表里都会被
    /// 截断成不可读的片段；Docker Desktop 与 Portainer 都是「短名做主标题、完整
    /// 引用留在详情」。代价是同一行看不到来源仓库，因此详情基础信息与起容器时
    /// 填的镜像参数始终使用 [`Self::display_name`] 的完整引用。
    pub fn short_display_name(&self) -> String {
        self.tags
            .first()
            .map(|tag| shorten_reference(tag))
            .unwrap_or_else(|| short_id(&self.id))
    }
}

/// 把镜像引用缩短为可读主干：去掉仓库主机段与官方 `library/` 前缀。
///
/// 判定沿用 Docker 自己的规则：只有第一段含 `.`、`:` 或等于 `localhost` 时才是
/// 仓库主机，否则它是命名空间（如 `ghcr.io/foo/bar` 的 `foo`）必须保留。
fn shorten_reference(reference: &str) -> String {
    let rest = match reference.split_once('/') {
        Some((head, tail)) if head.contains('.') || head.contains(':') || head == "localhost" => {
            tail
        }
        _ => reference,
    };
    rest.strip_prefix("library/").unwrap_or(rest).to_string()
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
    /// 健康检查结果（`starting`/`healthy`/`unhealthy`）；镜像没写 HEALTHCHECK 时为 `None`。
    #[serde(default)]
    pub health: Option<String>,
    #[serde(default)]
    pub restart_policy: Option<String>,
    #[serde(default)]
    pub ports: Vec<ContainerPort>,
    #[serde(default)]
    pub mounts: Vec<ContainerMount>,
    #[serde(default)]
    pub compose_project: Option<String>,
    /// 上一次退出是否由内核 OOM 终止：统计页把它当作告警横幅。
    #[serde(default)]
    pub oom_killed: bool,
    /// 可写层占用字节（inspect 带 `size=1` 时引擎才计算）。
    #[serde(default)]
    pub size_rw: u64,
    /// 镜像层合计字节（同上）。
    #[serde(default)]
    pub size_root_fs: u64,
    /// 镜像声明的暴露端口（`80/tcp`）：没有发布映射时端口卡片仍能显示。
    #[serde(default)]
    pub exposed_ports: Vec<String>,
    /// 原始 inspect JSON（格式化后），用于“高级信息”折叠区。
    #[serde(default)]
    pub inspect: String,
}

impl ContainerDetails {
    /// 健康状态展示文本；没有健康检查时给「未配置」，避免界面留白看不出新。
    pub fn health_label(&self) -> String {
        self.health.clone().unwrap_or_else(|| "none".to_string())
    }

    /// 是否处于「不健康」，界面据此决定状态点颜色。
    pub fn is_unhealthy(&self) -> bool {
        self.health.as_deref() == Some("unhealthy")
    }
}

/// 容器资源统计快照。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ContainerStats {
    /// CPU 占用百分比（0-100，按全部核心归一）。
    pub cpu_percent: f32,
    /// 逻辑核心数；界面用它显示「16 核」。
    #[serde(default)]
    pub cpu_count: u32,
    /// 每个逻辑核心的占用百分比；引擎不回报时为空。
    #[serde(default)]
    pub cpu_cores: Vec<f32>,
    /// 宿主机平均负载（1/5/15 分钟）。容器共享宿主内核，这不是容器自己的负载。
    #[serde(default)]
    pub host_load: Option<(f32, f32, f32)>,
    pub memory_used: u64,
    pub memory_limit: u64,
    /// 内存占用百分比（0-100）。
    pub memory_percent: f32,
    /// 常驻内存（RSS）；cgroup 未提供时为 0。
    #[serde(default)]
    pub memory_rss: u64,
    /// 页缓存：cgroup v1 取 `cache`，v2 取 `inactive_file`（Docker 即用后者）。
    #[serde(default)]
    pub memory_cache: u64,
    pub network_rx: u64,
    pub network_tx: u64,
    /// 每块网卡的累计收发：界面按相邻采样差值算速率。
    #[serde(default)]
    pub networks: Vec<ContainerInterfaceIo>,
    pub block_read: u64,
    pub block_write: u64,
    /// 按设备聚合的累计块 IO。
    #[serde(default)]
    pub block_devices: Vec<ContainerBlockDeviceIo>,
    /// 进程/线程数。
    pub pids: u64,
}

/// 单块网卡的累计收发字节。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ContainerInterfaceIo {
    pub name: String,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
}

/// 单个块设备的累计读写字节。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ContainerBlockDeviceIo {
    /// `major:minor`，例如 `8:0`。
    pub device: String,
    pub read_bytes: u64,
    pub write_bytes: u64,
}

/// 容器内的一个进程（来自 `GET /containers/{id}/top`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ContainerProcess {
    pub pid: String,
    pub user: String,
    /// CPU 占用百分比（`ps aux` 的 `%CPU`）；容器内 ps 不提供时为空。
    #[serde(default)]
    pub cpu_percent: f32,
    /// 内存占用百分比（`ps aux` 的 `%MEM`）。
    #[serde(default)]
    pub mem_percent: f32,
    pub command: String,
}

/// 镜像详情（基础信息页签）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerImageDetails {
    pub id: String,
    #[serde(default)]
    pub tags: Vec<String>,
    /// 仓库摘要（`名字@sha256:…`）：同一镜像被多个 tag 指向时，只有这里能看清来源。
    #[serde(default)]
    pub repo_digests: Vec<String>,
    /// 父镜像 ID；OCI 镜像通常为空串。
    #[serde(default)]
    pub parent: String,
    /// 解包后的虚拟大小（字节）；引擎不回报时为 0。
    #[serde(default)]
    pub virtual_size: u64,
    pub size: u64,
    pub created: i64,
    #[serde(default)]
    pub os: Option<String>,
    #[serde(default)]
    pub architecture: Option<String>,
    /// 层数量；引擎响应缺失时为 0。
    #[serde(default)]
    pub layers: usize,
    /// 每一层的 digest（自底向上）：详情页逐层展示，只有一个数字太抽象。
    #[serde(default)]
    pub layer_digests: Vec<String>,
    /// 原始 inspect JSON：界面「原始 JSON」视图用，引擎未返回时为空串。
    #[serde(default)]
    pub inspect: String,
    /// 镜像构建历史层。
    #[serde(default)]
    pub history: Vec<ContainerImageLayer>,
}

/// 镜像单层构建历史记录。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ContainerImageLayer {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub created: i64,
    #[serde(default)]
    pub created_by: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub comment: String,
    #[serde(default)]
    pub tags: Vec<String>,
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
    /// 驱动选项（`local` 的 `type`/`device` 等）：排查卷为什么挂不上时要看。
    #[serde(default)]
    pub options: Vec<(String, String)>,
}

/// 数据卷详情。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ContainerVolumeDetails {
    pub summary: ContainerVolumeSummary,
    /// 正在使用该卷的容器名。
    #[serde(default)]
    pub used_by: Vec<String>,
    /// 原始 inspect JSON：界面「原始 JSON」视图用。
    #[serde(default)]
    pub inspect: String,
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
    /// 仅宿主机内部可见（`--internal`）。
    #[serde(default)]
    pub internal: bool,
    /// 允许运行中的容器动态接入（`--attachable`）。
    #[serde(default)]
    pub attachable: bool,
    /// 启用了 IPv6 段。
    #[serde(default)]
    pub enable_ipv6: bool,
    /// 网络标签：排查编排自动创建的网络时要看归属。
    #[serde(default)]
    pub labels: Vec<(String, String)>,
    /// 原始 inspect JSON：界面「原始 JSON」视图用。
    #[serde(default)]
    pub inspect: String,
}

/// 一次清理（prune）的结果。
///
/// 设计原理（Why）：引擎的 prune 只告诉调用方删了哪些名字与回收了多少字节，
/// 具体删了几个必须回给用户看（否则「清理未使用卷」点下去毫无反馈，无法判断
/// 是本来就没得清还是没清成）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerPruneReport {
    /// 被删除的条目名（卷名或镜像摘要）。
    #[serde(default)]
    pub deleted: Vec<String>,
    /// 引擎报告的回收字节数。
    #[serde(default)]
    pub reclaimed: u64,
}

/// 编排项目命令。
///
/// 设计原理（Why）：引擎没有 Compose 的 REST 接口，这两个动作只能由 CLI 完成；
/// 界面把它做成枚举而不是字符串，避免把任意参数拼进子进程。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComposeAction {
    /// 按 compose 文件重新起（幂等：已存在的容器由 CLI 自己判断）。
    Up,
    /// 停止并删除该项目创建的容器与网络。
    Down,
}

impl ComposeAction {
    /// 界面与日志用的人类可读名。
    pub fn label(self) -> &'static str {
        match self {
            Self::Up => "启动",
            Self::Down => "停止并删除",
        }
    }

    /// 子命令名。
    pub fn subcommand(self) -> &'static str {
        match self {
            Self::Up => "up",
            Self::Down => "down",
        }
    }

    /// 子命令及其参数。
    pub fn args(self) -> &'static [&'static str] {
        match self {
            Self::Up => &["up", "-d"],
            Self::Down => &["down"],
        }
    }

    /// `down` 会删容器与网络，属于破坏性动作。
    pub fn is_dangerous(self) -> bool {
        matches!(self, Self::Down)
    }
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum ContainerLifecycleAction {
    Start,
    Stop,
    Restart,
    Pause,
    Unpause,
    Kill {
        #[serde(default)]
        signal: Option<String>,
    },
    Remove {
        #[serde(default)]
        force: bool,
    },
}

impl ContainerLifecycleAction {
    /// 日志与 UI 使用的动作名。
    pub fn label(&self) -> &'static str {
        match self {
            Self::Start => "启动",
            Self::Stop => "停止",
            Self::Restart => "重启",
            Self::Pause => "暂停",
            Self::Unpause => "恢复",
            Self::Kill { .. } => "强制终止",
            Self::Remove { .. } => "删除",
        }
    }
}

/// 容器动态更新配置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ContainerUpdateSpec {
    /// 内存上限（字节）；None 表示不修改。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_bytes: Option<u64>,
    /// CPU 配额（以 1e9 为 1 个核）；None 表示不修改。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nano_cpus: Option<u64>,
    /// 重启策略；None 表示不修改。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restart_policy: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::{ContainerImageSummary, shorten_reference};

    #[test]
    fn reference_keeps_namespace_and_drops_registry() {
        // 加速仓库主机与官方 library/ 都剥掉，只留可读主干。
        assert_eq!(
            shorten_reference("docker.m.daocloud.io/library/busybox:1.36"),
            "busybox:1.36"
        );
        // 第一段不含点与冒号时是命名空间，不能当成主机剥掉。
        assert_eq!(shorten_reference("foo/bar:latest"), "foo/bar:latest");
        assert_eq!(
            shorten_reference("ghcr.io/foo/bar:latest"),
            "foo/bar:latest",
            "主机段可剥，命名空间要留"
        );
        assert_eq!(shorten_reference("localhost:5000/app"), "app");
        assert_eq!(shorten_reference("busybox"), "busybox", "无斜杠时原样返回");
    }

    #[test]
    fn image_short_name_falls_back_to_id_when_untagged() {
        let dangling = ContainerImageSummary {
            id: "sha256:abcdef1234567890".to_string(),
            tags: Vec::new(),
            size: 0,
            created: 0,
        };
        assert_eq!(dangling.display_name(), dangling.short_display_name());
        let tagged = ContainerImageSummary {
            id: "sha256:abcdef1234567890".to_string(),
            tags: vec!["docker.m.daocloud.io/library/nginx:1.27".to_string()],
            size: 0,
            created: 0,
        };
        assert_eq!(tagged.short_display_name(), "nginx:1.27");
        assert_eq!(
            tagged.display_name(),
            "docker.m.daocloud.io/library/nginx:1.27",
            "完整引用仍然是起容器与删除时用的值"
        );
    }
}
