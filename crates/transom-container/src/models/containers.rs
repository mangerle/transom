//! 容器列表、检视、统计之外的容器线格式结构体与领域模型转换。

use super::{null_as_empty, parse_rfc3339_secs};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use transom_core::{
    ContainerDetails, ContainerFileSystemChange, ContainerHealthDetails, ContainerHealthLog,
    ContainerMount, ContainerPathChangeKind, ContainerPort, ContainerState, ContainerSummary,
    short_id,
};

/// `GET /containers/json` 列表项。
#[derive(Debug, Clone, Deserialize)]
pub struct ContainerListItem {
    #[serde(rename = "Id")]
    pub id: String,
    #[serde(rename = "Names", default)]
    pub names: Vec<String>,
    #[serde(rename = "Image", default)]
    pub image: String,
    #[serde(rename = "Created", default)]
    pub created: i64,
    #[serde(rename = "Ports", default)]
    pub ports: Vec<PortItem>,
    #[serde(rename = "State", default)]
    pub state: String,
    #[serde(rename = "Status", default)]
    pub status: String,
    #[serde(rename = "Labels", default, deserialize_with = "null_as_empty")]
    pub labels: HashMap<String, String>,
    #[serde(rename = "Mounts", default)]
    pub mounts: Vec<MountItem>,
}

/// 挂载项（列表与 inspect 共用）。
#[derive(Debug, Clone, Deserialize, Default)]
pub struct MountItem {
    #[serde(rename = "Type", default)]
    pub mount_type: String,
    #[serde(rename = "Name", default)]
    pub name: Option<String>,
    #[serde(rename = "Source", default)]
    pub source: String,
    #[serde(rename = "Destination", default)]
    pub destination: String,
    #[serde(rename = "Mode", default)]
    pub mode: String,
    #[serde(rename = "RW", default)]
    pub rw: bool,
}

impl MountItem {
    pub fn into_mount(self) -> ContainerMount {
        ContainerMount {
            kind: self.mount_type,
            name: self.name,
            source: self.source,
            destination: self.destination,
            mode: self.mode,
            rw: self.rw,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct PortItem {
    #[serde(rename = "IP", default)]
    pub ip: Option<String>,
    #[serde(rename = "PrivatePort", default)]
    pub private_port: u16,
    #[serde(rename = "PublicPort", default)]
    pub public_port: Option<u16>,
    #[serde(rename = "Type", default)]
    pub port_type: String,
}

impl ContainerListItem {
    /// 列表主名：Docker 会在名称前加 `/`。
    pub fn display_name(&self) -> String {
        self.names
            .first()
            .map(|name| name.trim_start_matches('/').to_string())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| short_id(&self.id))
    }

    pub fn into_summary(self) -> ContainerSummary {
        let name = self.display_name();
        let ports = self
            .ports
            .into_iter()
            .map(|port| ContainerPort {
                private_port: port.private_port,
                public_port: port.public_port,
                ip: port.ip,
                protocol: port.port_type,
            })
            .collect();
        let compose_project = self
            .labels
            .get("com.docker.compose.project")
            .cloned()
            .filter(|value| !value.is_empty());
        let compose_working_dir = self
            .labels
            .get("com.docker.compose.project.working_dir")
            .cloned()
            .filter(|value| !value.is_empty());
        let compose_config_files = self
            .labels
            .get("com.docker.compose.project.config_files")
            .map(|value| {
                value
                    .split(',')
                    .map(str::trim)
                    .filter(|item| !item.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        ContainerSummary {
            id: self.id,
            name,
            image: self.image,
            state: ContainerState::from_api(&self.state),
            status: self.status,
            created: self.created,
            ports,
            compose_project,
            compose_working_dir,
            compose_config_files,
            mounts: self.mounts.into_iter().map(MountItem::into_mount).collect(),
        }
    }
}

/// `GET /containers/{id}/json` 中本模块关心的字段。
#[derive(Debug, Clone, Deserialize, Default)]
pub struct ContainerInspect {
    #[serde(rename = "Id", default)]
    pub id: Option<String>,
    #[serde(rename = "Name", default)]
    pub name: Option<String>,
    #[serde(rename = "Created", default)]
    pub created: Option<String>,
    #[serde(rename = "Image", default)]
    pub image: Option<String>,
    #[serde(rename = "Config", default)]
    pub config: Option<ContainerConfig>,
    #[serde(rename = "State", default)]
    pub state: Option<InspectState>,
    #[serde(rename = "HostConfig", default)]
    pub host_config: Option<HostConfig>,
    #[serde(rename = "Mounts", default)]
    pub mounts: Vec<MountItem>,
    #[serde(rename = "NetworkSettings", default)]
    pub network_settings: Option<NetworkSettings>,
    /// 可写层 / 镜像层占用；只有 `?size=1` 时引擎才计算。
    #[serde(rename = "SizeRw", default)]
    pub size_rw: Option<u64>,
    #[serde(rename = "SizeRootFs", default)]
    pub size_root_fs: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ContainerConfig {
    /// TTY 容器的日志是裸流；非 TTY 才是 8 字节多路复用流。
    #[serde(rename = "Tty", default)]
    pub tty: bool,
    #[serde(rename = "Image", default)]
    pub image: Option<String>,
    #[serde(rename = "Cmd", default)]
    pub cmd: Option<Vec<String>>,
    #[serde(rename = "Entrypoint", default)]
    pub entrypoint: Option<Vec<String>>,
    #[serde(rename = "Env", default)]
    pub env: Option<Vec<String>>,
    #[serde(rename = "User", default)]
    pub user: Option<String>,
    #[serde(rename = "WorkingDir", default)]
    pub working_dir: Option<String>,
    #[serde(rename = "Labels", default, deserialize_with = "null_as_empty")]
    pub labels: HashMap<String, String>,
    /// 镜像声明的暴露端口：键形如 `80/tcp`，值恒为 null。
    #[serde(rename = "ExposedPorts", default, deserialize_with = "null_as_empty")]
    pub exposed_ports: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct InspectState {
    #[serde(rename = "Status", default)]
    pub status: Option<String>,
    #[serde(rename = "OOMKilled", default)]
    pub oom_killed: bool,
    #[serde(rename = "ExitCode", default)]
    pub exit_code: Option<i64>,
    #[serde(rename = "StartedAt", default)]
    pub started_at: Option<String>,
    #[serde(rename = "FinishedAt", default)]
    pub finished_at: Option<String>,
    #[serde(rename = "Health", default)]
    pub health: Option<HealthInfo>,
}

/// `State.Health`：只有镜像声明了 HEALTHCHECK 才出现。
#[derive(Debug, Clone, Deserialize, Default)]
pub struct HealthInfo {
    #[serde(rename = "Status", default)]
    pub status: Option<String>,
    #[serde(rename = "FailingStreak", default)]
    pub failing_streak: i64,
    #[serde(rename = "Log", default)]
    pub log: Vec<HealthLogItem>,
}

/// 健康检查单次执行日志。
#[derive(Debug, Clone, Deserialize, Default)]
pub struct HealthLogItem {
    #[serde(rename = "Start", default)]
    pub start: String,
    #[serde(rename = "End", default)]
    pub end: String,
    #[serde(rename = "ExitCode", default)]
    pub exit_code: i64,
    #[serde(rename = "Output", default)]
    pub output: String,
}

impl HealthInfo {
    pub fn into_health_details(self) -> Option<ContainerHealthDetails> {
        let status = self.status.filter(|s| !s.is_empty())?;
        let log = self
            .log
            .into_iter()
            .map(|item| ContainerHealthLog {
                start: item.start,
                end: item.end,
                exit_code: item.exit_code,
                output: item.output,
            })
            .collect();
        Some(ContainerHealthDetails {
            status,
            failing_streak: self.failing_streak,
            log,
        })
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct HostConfig {
    #[serde(rename = "RestartPolicy", default)]
    pub restart_policy: Option<RestartPolicy>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct RestartPolicy {
    #[serde(rename = "Name", default)]
    pub name: Option<String>,
}

/// `POST /containers/{id}/update` 请求体。
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "PascalCase")]
pub struct ContainerUpdateRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory: Option<u64>,
    #[serde(rename = "NanoCPUs", skip_serializing_if = "Option::is_none")]
    pub nano_cpus: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restart_policy: Option<RestartPolicyUpdate>,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "PascalCase")]
pub struct RestartPolicyUpdate {
    #[serde(rename = "Name")]
    pub name: String,
}

/// `GET /containers/{id}/changes` 响应项。
#[derive(Debug, Clone, Deserialize)]
pub struct ChangeItem {
    #[serde(rename = "Path")]
    pub path: String,
    #[serde(rename = "Kind")]
    pub kind: u8,
}

impl ChangeItem {
    pub fn into_change(self) -> ContainerFileSystemChange {
        let kind = match self.kind {
            1 => ContainerPathChangeKind::Added,
            2 => ContainerPathChangeKind::Deleted,
            _ => ContainerPathChangeKind::Modified,
        };
        ContainerFileSystemChange {
            path: self.path,
            kind,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct NetworkSettings {
    /// 形如 `{"80/tcp": [{"HostIp": "0.0.0.0", "HostPort": "8080"}]}`。
    #[serde(rename = "Ports", default, deserialize_with = "null_as_empty")]
    pub ports: HashMap<String, Option<Vec<PortBinding>>>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct PortBinding {
    #[serde(rename = "HostIp", default)]
    pub host_ip: Option<String>,
    #[serde(rename = "HostPort", default)]
    pub host_port: Option<String>,
}

impl ContainerInspect {
    /// 转换为界面使用的详情；`raw` 是原始 inspect JSON（已格式化）。
    pub fn into_details(self, raw: String) -> ContainerDetails {
        let config = self.config.unwrap_or_default();
        let state = self.state.unwrap_or_default();
        // 顶层 `Image` 是镜像 ID，`Config.Image` 才是仓库引用；两者不能混用。
        let image_id = self.image.unwrap_or_default();
        let image = config
            .image
            .filter(|image| !image.is_empty())
            .unwrap_or_else(|| image_id.clone());
        let command = config.cmd.as_deref().unwrap_or_default().join(" ");
        let entrypoint = config.entrypoint.unwrap_or_default();
        let restart_policy = self
            .host_config
            .and_then(|host| host.restart_policy)
            .and_then(|policy| policy.name)
            .filter(|name| name != "no");
        let ports = self
            .network_settings
            .map(|settings| settings.ports)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(key, bindings)| {
                let (port, protocol) = key.split_once('/')?;
                let private_port = port.parse::<u16>().ok()?;
                let binding = bindings.as_deref().and_then(|items| items.first());
                Some(ContainerPort {
                    private_port,
                    public_port: binding
                        .and_then(|binding| binding.host_port.as_deref())
                        .and_then(|port| port.parse::<u16>().ok()),
                    ip: binding.and_then(|binding| binding.host_ip.clone()),
                    protocol: protocol.to_string(),
                })
            })
            .collect();
        ContainerDetails {
            id: self.id.unwrap_or_default(),
            name: self
                .name
                .unwrap_or_default()
                .trim_start_matches('/')
                .to_string(),
            image,
            image_id,
            command,
            entrypoint,
            user: config.user.filter(|user| !user.is_empty()),
            working_dir: config.working_dir.filter(|dir| !dir.is_empty()),
            env: config.env.unwrap_or_default(),
            state: ContainerState::from_api(state.status.as_deref().unwrap_or_default()),
            status: state.status.clone().unwrap_or_default(),
            health: state
                .health
                .as_ref()
                .and_then(|health| health.status.clone())
                .filter(|status| !status.is_empty()),
            health_details: state.health.and_then(HealthInfo::into_health_details),
            created: parse_rfc3339_secs(self.created.as_deref()).unwrap_or(0),
            started_at: state.started_at,
            finished_at: state.finished_at,
            exit_code: state.exit_code,
            restart_policy,
            ports,
            mounts: self.mounts.into_iter().map(MountItem::into_mount).collect(),
            compose_project: config.labels.get("com.docker.compose.project").cloned(),
            oom_killed: state.oom_killed,
            size_rw: self.size_rw.unwrap_or(0),
            size_root_fs: self.size_root_fs.unwrap_or(0),
            exposed_ports: {
                let mut ports: Vec<String> = config.exposed_ports.keys().cloned().collect();
                ports.sort();
                ports
            },
            inspect: raw,
        }
    }
}

/// `POST /containers/prune` 响应。
#[derive(Debug, Clone, Deserialize, Default)]
pub struct ContainersPruneResponse {
    #[serde(
        rename = "ContainersDeleted",
        default,
        deserialize_with = "null_as_empty"
    )]
    pub containers_deleted: Vec<String>,
    #[serde(rename = "SpaceReclaimed", default)]
    pub space_reclaimed: u64,
}
