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
