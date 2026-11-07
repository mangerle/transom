//! 容器状态、摘要、详情、挂载、健康检查与文件变更条目。

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

/// 容器挂载。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ContainerMount {
    /// `bind` / `volume` / `tmpfs` 等。
    pub kind: String,
    /// 命名卷名称；`bind` 挂载时为空。
    ///
    /// 设计原理（Why）：Docker 的列表与检视接口在 `Source` 中返回的是宿主挂载点
    /// （如 `/var/lib/docker/volumes/app-data/_data`），只有 `Name` 才是卷名。
    /// 判断“卷是否在用”以及按卷复用容器都必须使用卷名，不能把宿主路径当卷名。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// 宿主侧来源（卷名或宿主路径）。
    pub source: String,
    /// 容器内目标路径。
    pub destination: String,
    #[serde(default)]
    pub mode: String,
    #[serde(default)]
    pub rw: bool,
}

impl ContainerMount {
    /// 返回命名卷的真实卷名；非卷挂载返回 `None`。
    ///
    /// 设计原理（Why）：引擎对命名卷同时给出 `Name` 与宿主挂载点 `Source`，
    /// 业务层只需要卷名；`Name` 缺失时退回 `Source`，兼容把卷名直接放在
    /// `Source` 的引擎或旧数据，避免历史配置失效。
    pub fn volume_name(&self) -> Option<&str> {
        if self.kind != "volume" {
            return None;
        }
        self.name
            .as_deref()
            .filter(|name| !name.is_empty())
            .or_else(|| (!self.source.is_empty()).then_some(self.source.as_str()))
    }
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
    /// 健康检查诊断详情（状态、连续失败次数与检查日志）。
    #[serde(default)]
    pub health_details: Option<ContainerHealthDetails>,
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

/// 容器健康检查单次执行日志。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ContainerHealthLog {
    #[serde(default)]
    pub start: String,
    #[serde(default)]
    pub end: String,
    #[serde(default)]
    pub exit_code: i64,
    #[serde(default)]
    pub output: String,
}

/// 容器健康检查诊断详情。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ContainerHealthDetails {
    pub status: String,
    #[serde(default)]
    pub failing_streak: i64,
    #[serde(default)]
    pub log: Vec<ContainerHealthLog>,
}

/// 容器文件系统变更类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContainerPathChangeKind {
    Modified,
    Added,
    Deleted,
}

impl ContainerPathChangeKind {
    pub fn badge_char(self) -> &'static str {
        match self {
            Self::Added => "A",
            Self::Deleted => "D",
            Self::Modified => "C",
        }
    }
}

/// 容器单条文件系统变更记录。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerFileSystemChange {
    pub path: String,
    pub kind: ContainerPathChangeKind,
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
