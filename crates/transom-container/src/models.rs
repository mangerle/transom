//! Docker/Podman REST API 的线格式结构体与到领域模型的转换。
//!
//! 原则（Why）：外部 JSON 只在本模块出现一次，立即收敛为
//! `transom-core` 的强类型；引擎字段增删或大小写差异不会扩散到控制器与界面。

use serde::{Deserialize, Serialize};
use serde::de::Deserializer;
use std::collections::HashMap;
use transom_core::{
    ContainerBlockDeviceIo, ContainerDetails, ContainerImageDetails, ContainerImageLayer,
    ContainerImageSummary, ContainerInterfaceIo, ContainerMount, ContainerNetworkAttachment,
    ContainerNetworkDetails, ContainerNetworkSummary, ContainerPort, ContainerProcess,
    ContainerState, ContainerStats, ContainerSummary, ContainerVolumeSummary, short_id,
};

/// 把线格式里的 `null` 收敛成空集合。
///
/// 设计与原因（Why）：引擎对「无标签」的数据卷返回 `"Labels": null` 而不是 `{}`，
/// 而 `#[serde(default)]` 只兜底「键缺失」，键存在但为 null 会直接报类型错误。
/// 这类字段只用于展示，空集合就是合法语义，不值得为它新增错误分支。
fn null_as_empty<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

/// `GET /version` 响应（只取需要的字段）。
#[derive(Debug, Clone, Deserialize, Default)]
pub struct VersionResponse {
    #[serde(rename = "Version", default)]
    pub version: Option<String>,
    #[serde(rename = "ApiVersion", default)]
    pub api_version: Option<String>,
    #[serde(rename = "Platform", default)]
    pub platform: Option<PlatformInfo>,
    #[serde(rename = "Components", default)]
    pub components: Option<Vec<ComponentInfo>>,
    /// 引擎运行系统；`Platform.Name` 为空时用于兜底展示。
    #[serde(rename = "Os", default)]
    pub os: Option<String>,
    /// 引擎架构；与 `Os` 一起构成兜底平台名。
    #[serde(rename = "Arch", default)]
    pub arch: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct PlatformInfo {
    #[serde(rename = "Name", default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ComponentInfo {
    #[serde(rename = "Name", default)]
    pub name: Option<String>,
}

impl VersionResponse {
    /// 识别是否为 Podman；识别不出时沿用调用方声明的引擎种类。
    pub fn is_podman(&self) -> bool {
        let platform = self
            .platform
            .as_ref()
            .and_then(|platform| platform.name.as_deref())
            .unwrap_or_default();
        if platform.to_ascii_lowercase().contains("podman") {
            return true;
        }
        self.components
            .as_deref()
            .unwrap_or_default()
            .iter()
            .filter_map(|component| component.name.as_deref())
            .any(|name| name.to_ascii_lowercase().contains("podman"))
    }

    /// 界面展示的平台名。
    ///
    /// 兜底次序（Why）：真机 Docker 29 的 `/version` 把 `Platform.Name` 返回成
    /// 空串（键存在但无内容），直接展示会得到一片空白；因此先取非空的平台名，
    /// 再回落到 `os/arch`，最后才是通用文案。
    pub fn platform_name(&self) -> String {
        non_blank(
            self.platform
                .as_ref()
                .and_then(|platform| platform.name.as_deref()),
        )
        .unwrap_or_else(|| {
            match (
                non_blank(self.os.as_deref()),
                non_blank(self.arch.as_deref()),
            ) {
                (Some(os), Some(arch)) => format!("{os}/{arch}"),
                (Some(os), None) | (None, Some(os)) => os,
                (None, None) => "Docker/Podman".to_string(),
            }
        })
    }
}

/// 取非空白文本；键存在但为空串时按缺失处理。
fn non_blank(value: Option<&str>) -> Option<String> {
    let trimmed = value?.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

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

/// `GET /images/json` 列表项。
#[derive(Debug, Clone, Deserialize)]
pub struct ImageListItem {
    #[serde(rename = "Id")]
    pub id: String,
    /// 悬空镜像为 null；用 `Option` 兜住。
    #[serde(rename = "RepoTags", default)]
    pub repo_tags: Option<Vec<String>>,
    #[serde(rename = "Created", default)]
    pub created: i64,
    #[serde(rename = "Size", default)]
    pub size: u64,
}

impl ImageListItem {
    pub fn into_summary(self) -> ContainerImageSummary {
        ContainerImageSummary {
            id: self.id,
            tags: self.repo_tags.unwrap_or_default(),
            size: self.size,
            created: self.created,
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
            image: config.image.or(self.image).unwrap_or_default(),
            image_id: String::new(),
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

/// RFC3339 时间转 Unix 秒；解析失败返回 `None`。
pub fn parse_rfc3339_secs(value: Option<&str>) -> Option<i64> {
    let value = value?;
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|datetime| datetime.timestamp())
}

/// `POST /images/prune` 响应。
#[derive(Debug, Clone, Deserialize, Default)]
pub struct ImagePruneResponse {
    /// 没有可删条目时引擎返回 `null`，不能交给 `default` 硬接。
    #[serde(rename = "ImagesDeleted", default, deserialize_with = "null_as_empty")]
    pub images_deleted: Vec<ImagePruneEntry>,
    #[serde(rename = "SpaceReclaimed", default)]
    pub space_reclaimed: u64,
}

/// 单条镜像清理记录：引擎按动作回报，取消标签与删除镜像是两种条目。
#[derive(Debug, Clone, Deserialize, Default)]
pub struct ImagePruneEntry {
    #[serde(rename = "Untagged", default)]
    pub untagged: Option<String>,
    #[serde(rename = "Deleted", default)]
    pub deleted: Option<String>,
}

/// `POST /volumes/prune` 响应。
#[derive(Debug, Clone, Deserialize, Default)]
pub struct VolumePruneResponse {
    /// 同镜像清理：没有可删条目时是 `null`。
    #[serde(rename = "VolumesDeleted", default, deserialize_with = "null_as_empty")]
    pub volumes_deleted: Vec<String>,
    #[serde(rename = "SpaceReclaimed", default)]
    pub space_reclaimed: u64,
}

/// `POST /networks/prune` 响应。
#[derive(Debug, Clone, Deserialize, Default)]
pub struct NetworkPruneResponse {
    /// 清理掉的网络名列表；没有可删条目时是 `null`。
    #[serde(rename = "NetworksDeleted", default, deserialize_with = "null_as_empty")]
    pub networks_deleted: Vec<String>,
}

/// `GET /volumes` 响应。
#[derive(Debug, Clone, Deserialize, Default)]
pub struct VolumeListResponse {
    #[serde(rename = "Volumes", default)]
    pub volumes: Vec<VolumeItem>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct VolumeItem {
    #[serde(rename = "Name", default)]
    pub name: String,
    #[serde(rename = "Driver", default)]
    pub driver: String,
    #[serde(rename = "Mountpoint", default)]
    pub mountpoint: String,
    #[serde(rename = "CreatedAt", default)]
    pub created_at: Option<String>,
    #[serde(rename = "Labels", default, deserialize_with = "null_as_empty")]
    pub labels: HashMap<String, String>,
    #[serde(rename = "Options", default, deserialize_with = "null_as_empty")]
    pub options: HashMap<String, String>,
}

impl VolumeItem {
    pub fn into_summary(self) -> ContainerVolumeSummary {
        ContainerVolumeSummary {
            name: self.name,
            driver: self.driver,
            mountpoint: self.mountpoint,
            created: self.created_at,
            labels: sorted_pairs(self.labels),
            options: sorted_pairs(self.options),
        }
    }
}

/// 把引擎的 map 字段摊平成按 key 排序的列表：界面渲染与差分都要求稳定顺序。
fn sorted_pairs(map: HashMap<String, String>) -> Vec<(String, String)> {
    let mut pairs: Vec<(String, String)> = map.into_iter().collect();
    pairs.sort_by(|left, right| left.0.cmp(&right.0));
    pairs
}

/// `GET /networks` 列表项。
#[derive(Debug, Clone, Deserialize, Default)]
pub struct NetworkItem {
    #[serde(rename = "Id", default)]
    pub id: String,
    #[serde(rename = "Name", default)]
    pub name: String,
    #[serde(rename = "Driver", default)]
    pub driver: String,
    #[serde(rename = "Scope", default)]
    pub scope: String,
    #[serde(rename = "IPAM", default)]
    pub ipam: Option<Ipam>,
    #[serde(rename = "Containers", default)]
    pub containers: Option<HashMap<String, NetworkContainerItem>>,
    #[serde(rename = "Internal", default)]
    pub internal: bool,
    #[serde(rename = "Attachable", default)]
    pub attachable: bool,
    #[serde(rename = "EnableIPv6", default)]
    pub enable_ipv6: bool,
    /// 引擎在无标签时返回 `null`，`HashMap` 的 `default` 接不住显式 null。
    #[serde(rename = "Labels", default, deserialize_with = "null_as_empty")]
    pub labels: HashMap<String, String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Ipam {
    /// 内置网络（bridge/host/none）会返回 `"Config": null`；`Vec` 的 `default`
    /// 只覆盖字段缺失，接不住显式 null，曾经导致网络列表整体解析失败。
    #[serde(rename = "Config", default, deserialize_with = "null_as_empty")]
    pub config: Vec<IpamConfig>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct IpamConfig {
    #[serde(rename = "Subnet", default)]
    pub subnet: Option<String>,
    #[serde(rename = "Gateway", default)]
    pub gateway: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct NetworkContainerItem {
    #[serde(rename = "Name", default)]
    pub name: String,
    #[serde(rename = "IPv4Address", default)]
    pub ipv4_address: Option<String>,
}

impl NetworkItem {
    pub fn summary(&self) -> ContainerNetworkSummary {
        ContainerNetworkSummary {
            id: self.id.clone(),
            name: self.name.clone(),
            driver: self.driver.clone(),
            scope: self.scope.clone(),
            subnet: self
                .ipam
                .as_ref()
                .and_then(|ipam| ipam.config.first())
                .and_then(|config| config.subnet.clone()),
        }
    }

    /// 收敛为界面详情；`raw` 是引擎原始 JSON，供「原始 JSON」视图直接展示。
    pub fn into_details(self, raw: String) -> ContainerNetworkDetails {
        let summary = self.summary();
        let gateway = self
            .ipam
            .as_ref()
            .and_then(|ipam| ipam.config.first())
            .and_then(|config| config.gateway.clone());
        let mut connected: Vec<ContainerNetworkAttachment> = self
            .containers
            .unwrap_or_default()
            .into_iter()
            .map(|(id, item)| ContainerNetworkAttachment {
                id: short_id(&id),
                name: item.name,
                ipv4: item
                    .ipv4_address
                    .map(|address| address.trim_end_matches('/').to_string())
                    .filter(|address| !address.is_empty()),
            })
            .collect();
        connected.sort_by(|left, right| left.name.cmp(&right.name));
        ContainerNetworkDetails {
            summary,
            gateway,
            connected,
            internal: self.internal,
            attachable: self.attachable,
            enable_ipv6: self.enable_ipv6,
            labels: sorted_pairs(self.labels),
            inspect: raw,
        }
    }
}

/// `GET /images/{id}/json` 中本模块关心的字段。
#[derive(Debug, Clone, Deserialize, Default)]
pub struct ImageInspect {
    #[serde(rename = "Id", default)]
    pub id: String,
    #[serde(rename = "RepoTags", default)]
    pub repo_tags: Option<Vec<String>>,
    #[serde(rename = "Size", default)]
    pub size: u64,
    #[serde(rename = "Created", default)]
    pub created: Option<String>,
    #[serde(rename = "Os", default)]
    pub os: Option<String>,
    #[serde(rename = "Architecture", default)]
    pub architecture: Option<String>,
    #[serde(rename = "RootFS", default)]
    pub root_fs: Option<RootFs>,
    #[serde(rename = "RepoDigests", default)]
    pub repo_digests: Option<Vec<String>>,
    #[serde(rename = "ParentId", default)]
    pub parent_id: Option<String>,
    #[serde(rename = "VirtualSize", default)]
    pub virtual_size: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct RootFs {
    #[serde(rename = "Layers", default)]
    pub layers: Vec<String>,
}

impl ImageInspect {
    /// 收敛为界面详情；`raw` 是引擎原始 JSON，供「原始 JSON」视图直接展示。
    pub fn into_details(self, raw: String) -> ContainerImageDetails {
        let layer_digests = self.root_fs.map(|root| root.layers).unwrap_or_default();
        ContainerImageDetails {
            id: self.id,
            tags: self.repo_tags.unwrap_or_default(),
            repo_digests: self.repo_digests.unwrap_or_default(),
            parent: self.parent_id.unwrap_or_default(),
            virtual_size: self.virtual_size.unwrap_or(0),
            inspect: raw,
            size: self.size,
            created: parse_rfc3339_secs(self.created.as_deref()).unwrap_or(0),
            os: self.os,
            architecture: self.architecture,
            layers: layer_digests.len(),
            layer_digests,
            history: Vec::new(),
        }
    }
}

/// `GET /images/{name}/history` 响应单项。
#[derive(Debug, Clone, Deserialize, Default)]
pub struct ImageHistoryItem {
    #[serde(rename = "Id", default)]
    pub id: String,
    #[serde(rename = "Created", default)]
    pub created: i64,
    #[serde(rename = "CreatedBy", default)]
    pub created_by: String,
    #[serde(rename = "Tags", default)]
    pub tags: Option<Vec<String>>,
    #[serde(rename = "Size", default)]
    pub size: u64,
    #[serde(rename = "Comment", default)]
    pub comment: String,
}

impl ImageHistoryItem {
    pub fn into_layer(self) -> ContainerImageLayer {
        ContainerImageLayer {
            id: self.id,
            created: self.created,
            created_by: self.created_by,
            size: self.size,
            comment: self.comment,
            tags: self.tags.unwrap_or_default(),
        }
    }
}

/// `GET /containers/{id}/stats` 响应。
#[derive(Debug, Clone, Deserialize, Default)]
pub struct StatsResponse {
    #[serde(rename = "cpu_stats", default)]
    pub cpu_stats: CpuStats,
    #[serde(rename = "precpu_stats", default)]
    pub precpu_stats: CpuStats,
    #[serde(rename = "memory_stats", default)]
    pub memory_stats: MemoryStats,
    #[serde(rename = "networks", default)]
    pub networks: Option<HashMap<String, NetworkIo>>,
    #[serde(rename = "blkio_stats", default)]
    pub blkio_stats: Option<BlkioStats>,
    #[serde(rename = "pids_stats", default)]
    pub pids_stats: Option<PidsStats>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct CpuStats {
    #[serde(rename = "cpu_usage", default)]
    pub cpu_usage: CpuUsage,
    #[serde(rename = "system_cpu_usage", default)]
    pub system_cpu_usage: Option<u64>,
    #[serde(rename = "online_cpus", default)]
    pub online_cpus: Option<u32>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct CpuUsage {
    #[serde(rename = "total_usage", default)]
    pub total_usage: u64,
    #[serde(rename = "percpu_usage", default)]
    pub percpu_usage: Option<Vec<u64>>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct MemoryStats {
    #[serde(rename = "usage", default)]
    pub usage: u64,
    #[serde(rename = "limit", default)]
    pub limit: u64,
    /// 细分项（`rss`/`cache`/`inactive_file` 等）：统计页用它展示内存构成。
    #[serde(rename = "stats", default, deserialize_with = "null_as_empty")]
    pub stats: HashMap<String, u64>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct NetworkIo {
    #[serde(rename = "rx_bytes", default)]
    pub rx_bytes: u64,
    #[serde(rename = "tx_bytes", default)]
    pub tx_bytes: u64,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct BlkioStats {
    #[serde(rename = "io_service_bytes_recursive", default)]
    pub io_service_bytes_recursive: Option<Vec<BlkioEntry>>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct BlkioEntry {
    #[serde(rename = "op", default)]
    pub op: String,
    #[serde(rename = "value", default)]
    pub value: u64,
    #[serde(rename = "major", default)]
    pub major: Option<u16>,
    #[serde(rename = "minor", default)]
    pub minor: Option<u16>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct PidsStats {
    #[serde(rename = "current", default)]
    pub current: u64,
}

/// `GET /containers/{id}/top` 响应（`ps_args=aux`）。
#[derive(Debug, Clone, Deserialize, Default)]
pub struct TopResponse {
    #[serde(rename = "Titles", default, deserialize_with = "null_as_empty")]
    pub titles: Vec<String>,
    #[serde(rename = "Processes", default, deserialize_with = "null_as_empty")]
    pub processes: Vec<Vec<String>>,
}

impl TopResponse {
    /// 按标题定位列：不同镜像的 `ps` 列顺序可能不同，不能按下标硬取。
    pub fn into_processes(self) -> Vec<ContainerProcess> {
        let column = |name: &str| {
            self.titles
                .iter()
                .position(|title| title.eq_ignore_ascii_case(name))
        };
        let pid = column("PID");
        let user = column("USER");
        let cpu = column("%CPU");
        let mem = column("%MEM");
        let command = column("COMMAND")
            .or_else(|| column("COMMAND_AND_ARGS"))
            .or_else(|| column("ARGS"))
            .or_else(|| column("CMD"));
        self.processes
            .into_iter()
            .map(|row| {
                let cell = |index: Option<usize>| {
                    index
                        .and_then(|index| row.get(index))
                        .cloned()
                        .unwrap_or_default()
                };
                ContainerProcess {
                    pid: cell(pid),
                    user: cell(user),
                    cpu_percent: cell(cpu).parse().unwrap_or(0.0),
                    mem_percent: cell(mem).parse().unwrap_or(0.0),
                    command: command
                        .and_then(|index| row.get(index..).map(|tail| tail.join(" ")))
                        .unwrap_or_default(),
                }
            })
            .collect()
    }
}

impl StatsResponse {
    /// 计算界面展示的统计快照；`previous` 用于非流式响应的 CPU 差值兜底。
    pub fn to_stats(&self, previous: Option<&Self>) -> ContainerStats {
        let previous_cpu = if self.precpu_stats.cpu_usage.total_usage > 0 {
            &self.precpu_stats
        } else {
            previous
                .map(|sample| &sample.cpu_stats)
                .unwrap_or(&self.precpu_stats)
        };
        let cpu_delta = self
            .cpu_stats
            .cpu_usage
            .total_usage
            .saturating_sub(previous_cpu.cpu_usage.total_usage);
        let system_delta = self
            .cpu_stats
            .system_cpu_usage
            .unwrap_or(0)
            .saturating_sub(previous_cpu.system_cpu_usage.unwrap_or(0));
        let online_cpus = self
            .cpu_stats
            .online_cpus
            .map(u64::from)
            .or_else(|| {
                self.cpu_stats
                    .cpu_usage
                    .percpu_usage
                    .as_ref()
                    .map(|cores| cores.len() as u64)
            })
            .unwrap_or(1)
            .max(1);
        let cpu_percent = if system_delta > 0 {
            (cpu_delta as f64 / system_delta as f64 * online_cpus as f64 * 100.0) as f32
        } else {
            0.0
        };
        // 每核占用：用各核累计 jiffies 的差值除以单核系统时间增量。
        let mut cpu_cores = Vec::new();
        if system_delta > 0
            && let (Some(current), Some(previous)) = (
                self.cpu_stats.cpu_usage.percpu_usage.as_ref(),
                previous_cpu.cpu_usage.percpu_usage.as_ref(),
            )
            && current.len() == previous.len()
        {
            let per_core_system = system_delta as f64 / online_cpus as f64;
            cpu_cores = current
                .iter()
                .zip(previous)
                .map(|(now, before)| {
                    let delta = now.saturating_sub(*before) as f64;
                    (delta / per_core_system * 100.0).clamp(0.0, 100.0) as f32
                })
                .collect();
        }
        let memory_used = self.memory_stats.usage;
        let memory_limit = self.memory_stats.limit;
        let memory_percent = if memory_limit > 0 {
            (memory_used as f64 / memory_limit as f64 * 100.0) as f32
        } else {
            0.0
        };
        // cgroup v1 用 rss/cache，v2 用 anon/inactive_file，两套键都认。
        let memory_rss = self
            .memory_stats
            .stats
            .get("rss")
            .or_else(|| self.memory_stats.stats.get("anon"))
            .copied()
            .unwrap_or(0);
        let memory_cache = self
            .memory_stats
            .stats
            .get("cache")
            .or_else(|| self.memory_stats.stats.get("inactive_file"))
            .copied()
            .unwrap_or(0);
        let mut networks: Vec<ContainerInterfaceIo> = self
            .networks
            .as_ref()
            .map(|map| {
                map.iter()
                    .map(|(name, interface)| ContainerInterfaceIo {
                        name: name.clone(),
                        rx_bytes: interface.rx_bytes,
                        tx_bytes: interface.tx_bytes,
                    })
                    .collect()
            })
            .unwrap_or_default();
        networks.sort_by(|left, right| left.name.cmp(&right.name));
        let network_rx = networks
            .iter()
            .fold(0u64, |total, item| total.saturating_add(item.rx_bytes));
        let network_tx = networks
            .iter()
            .fold(0u64, |total, item| total.saturating_add(item.tx_bytes));
        let mut per_device: HashMap<(u16, u16), (u64, u64)> = HashMap::new();
        let (mut block_read, mut block_write) = (0u64, 0u64);
        if let Some(entries) = self
            .blkio_stats
            .as_ref()
            .and_then(|stats| stats.io_service_bytes_recursive.as_ref())
        {
            for entry in entries {
                match entry.op.as_str() {
                    "read" | "Read" => {
                        block_read = block_read.saturating_add(entry.value);
                        let slot = per_device
                            .entry((entry.major.unwrap_or(0), entry.minor.unwrap_or(0)))
                            .or_default();
                        slot.0 = slot.0.saturating_add(entry.value);
                    }
                    "write" | "Write" => {
                        block_write = block_write.saturating_add(entry.value);
                        let slot = per_device
                            .entry((entry.major.unwrap_or(0), entry.minor.unwrap_or(0)))
                            .or_default();
                        slot.1 = slot.1.saturating_add(entry.value);
                    }
                    _ => {}
                }
            }
        }
        let mut block_devices: Vec<ContainerBlockDeviceIo> = per_device
            .into_iter()
            .map(|((major, minor), (read, write))| ContainerBlockDeviceIo {
                device: format!("{major}:{minor}"),
                read_bytes: read,
                write_bytes: write,
            })
            .collect();
        block_devices.sort_by(|left, right| left.device.cmp(&right.device));
        ContainerStats {
            cpu_percent,
            cpu_count: online_cpus as u32,
            cpu_cores,
            host_load: None,
            memory_used,
            memory_limit,
            memory_percent,
            memory_rss,
            memory_cache,
            network_rx,
            network_tx,
            networks,
            block_read,
            block_write,
            block_devices,
            pids: self
                .pids_stats
                .as_ref()
                .map(|pids| pids.current)
                .unwrap_or(0),
        }
    }
}

/// `POST /containers/{id}/exec` 响应。
#[derive(Debug, Clone, Deserialize)]
pub struct ExecCreateResponse {
    #[serde(rename = "Id")]
    pub id: String,
}

/// `GET /exec/{id}/json` 响应。
#[derive(Debug, Clone, Deserialize, Default)]
pub struct ExecInspectResponse {
    #[serde(rename = "ExitCode", default)]
    pub exit_code: Option<i64>,
    #[serde(rename = "Running", default)]
    pub running: bool,
}

/// 镜像拉取事件；只用于识别流中的错误。
#[derive(Debug, Clone, Deserialize, Default)]
pub struct ImagePullEvent {
    #[serde(default)]
    pub error: Option<String>,
    #[serde(rename = "errorDetail", default)]
    pub error_detail: Option<PullErrorDetail>,
    #[serde(rename = "progressDetail", default)]
    pub progress_detail: Option<ProgressDetail>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct PullErrorDetail {
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ProgressDetail {
    #[serde(default)]
    pub current: Option<u64>,
    #[serde(default)]
    pub total: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::{
        ContainerInspect, ContainerListItem, ImageListItem, ImagePruneResponse, NetworkItem,
        NetworkSettings, StatsResponse, TopResponse, VersionResponse, VolumeItem,
        VolumePruneResponse,
    };
    use transom_core::ContainerState;

    #[test]
    fn container_list_item_tolerates_extra_fields_and_null_repo_tags() {
        let json = r#"[
            {
                "Id": "sha256:abc123",
                "Names": ["/web"],
                "Image": "nginx:latest",
                "ImageID": "sha256:def",
                "Created": 1700000000,
                "Ports": [{"IP": "0.0.0.0", "PrivatePort": 80, "PublicPort": 8080, "Type": "tcp"}],
                "Labels": {"com.example": "1"},
                "State": "running",
                "Status": "Up 3 hours",
                "Mounts": []
            }
        ]"#;
        let items: Vec<ContainerListItem> = serde_json::from_str(json).expect("列表必须能反序列化");
        let summary = items.into_iter().next().expect("至少一条").into_summary();
        assert_eq!(summary.name, "web");
        assert_eq!(summary.state, ContainerState::Running);
        assert_eq!(summary.ports_label(), "8080->80/tcp");

        let images = r#"[{"Id":"sha256:f00","RepoTags":null,"Created":1,"Size":10}]"#;
        let images: Vec<ImageListItem> = serde_json::from_str(images).expect("镜像必须能反序列化");
        let image = images.into_iter().next().expect("至少一个").into_summary();
        assert!(image.tags.is_empty());
        assert_eq!(image.display_name(), "f00");
    }

    /// 真机回归：内置网络的 `IPAM.Config` 与无标签网络的 `Labels` 会是显式 null，
    /// 不能让整个网络列表解析失败。
    #[test]
    fn network_list_tolerates_null_ipam_and_labels() {
        let json = r#"{
            "Id": "n1",
            "Name": "bridge",
            "Driver": "bridge",
            "Scope": "local",
            "IPAM": { "Driver": "default", "Options": null, "Config": null },
            "Labels": null,
            "Containers": null
        }"#;
        let item: NetworkItem =
            serde_json::from_str(json).expect("网络列表必须能反序列化 null 字段");
        let summary = item.summary();
        assert_eq!(summary.name, "bridge");
        assert!(summary.subnet.is_none(), "Config 为 null 时应按无子网处理");
    }

    /// 统计快照要能给出每核、分网卡与分设备明细：这几项是统计页卡片的数据来源。
    #[test]
    fn stats_report_cores_interfaces_and_devices() {
        let current: StatsResponse = serde_json::from_str(
            r#"{
                "cpu_stats": {
                    "cpu_usage": {"total_usage": 160000, "percpu_usage": [80000, 80000]},
                    "system_cpu_usage": 2000000,
                    "online_cpus": 2
                },
                "precpu_stats": {
                    "cpu_usage": {"total_usage": 100000, "percpu_usage": [50000, 50000]},
                    "system_cpu_usage": 1000000,
                    "online_cpus": 2
                },
                "memory_stats": {
                    "usage": 100,
                    "limit": 200,
                    "stats": {"rss": 60, "inactive_file": 20}
                },
                "networks": {"eth0": {"rx_bytes": 1000, "tx_bytes": 200}},
                "blkio_stats": {
                    "io_service_bytes_recursive": [
                        {"major": 8, "minor": 0, "op": "read", "value": 10},
                        {"major": 8, "minor": 0, "op": "write", "value": 20}
                    ]
                },
                "pids_stats": {"current": 3}
            }"#,
        )
        .expect("统计响应必须能反序列化");
        let previous: StatsResponse = serde_json::from_str(
            r#"{
                "cpu_stats": {
                    "cpu_usage": {"total_usage": 100000, "percpu_usage": [50000, 50000]},
                    "system_cpu_usage": 1000000,
                    "online_cpus": 2
                }
            }"#,
        )
        .expect("上一帧统计必须能反序列化");
        let stats = current.to_stats(Some(&previous));
        assert!((stats.cpu_percent - 12.0).abs() < 0.01, "总占用");
        assert_eq!(stats.cpu_cores.len(), 2);
        assert!((stats.cpu_cores[0] - 6.0).abs() < 0.01, "每核占用");
        assert_eq!(stats.memory_rss, 60);
        assert_eq!(stats.memory_cache, 20);
        assert_eq!(stats.networks[0].name, "eth0");
        assert_eq!(stats.networks[0].rx_bytes, 1000);
        assert_eq!(stats.block_devices[0].device, "8:0");
        assert_eq!(stats.block_devices[0].read_bytes, 10);
        assert_eq!(stats.block_devices[0].write_bytes, 20);
    }

    /// `docker top` 的列顺序不固定，按标题定位。
    #[test]
    fn top_response_maps_columns_by_title() {
        let top: TopResponse = serde_json::from_str(
            r#"{
                "Titles": ["PID", "USER", "%CPU", "%MEM", "COMMAND"],
                "Processes": [
                    ["1", "root", "0.0", "0.1", "sleep 3600"],
                    ["7", "nobody", "1.5", "0.2", "sh -c echo hi"]
                ]
            }"#,
        )
        .expect("top 响应必须能反序列化");
        let processes = top.into_processes();
        assert_eq!(processes.len(), 2);
        assert_eq!(processes[0].pid, "1");
        assert_eq!(processes[0].user, "root");
        assert!((processes[1].cpu_percent - 1.5).abs() < f32::EPSILON);
        assert_eq!(processes[1].command, "sh -c echo hi");
    }

    /// 清理接口在「没有可删条目」时返回 null 数组，不能因此报解析失败。
    #[test]
    fn prune_responses_tolerate_null_arrays() {
        let images: ImagePruneResponse =
            serde_json::from_str(r#"{"ImagesDeleted":null,"SpaceReclaimed":0}"#)
                .expect("镜像清理响应必须能反序列化");
        assert!(images.images_deleted.is_empty());
        let volumes: VolumePruneResponse =
            serde_json::from_str(r#"{"VolumesDeleted":null,"SpaceReclaimed":0}"#)
                .expect("卷清理响应必须能反序列化");
        assert!(volumes.volumes_deleted.is_empty());
    }

    #[test]
    fn version_detects_podman_from_platform() {
        let json = r#"{"Version":"5.2.0","ApiVersion":"1.41","Platform":{"Name":"Podman Engine"}}"#;
        let version: VersionResponse = serde_json::from_str(json).expect("版本必须能反序列化");
        assert!(version.is_podman());
        assert_eq!(version.platform_name(), "Podman Engine");
    }

    /// 真机回归：Docker 29 返回 `"Platform":{"Name":""}`，展示必须落到 `os/arch`，
    /// 且空平台名不能被误判成 Podman。
    #[test]
    fn blank_platform_falls_back_to_os_arch() {
        let json = r#"{"Version":"29.1.3","ApiVersion":"1.52","Platform":{"Name":""},"Os":"linux","Arch":"amd64","Components":[{"Name":"Engine"}]}"#;
        let version: VersionResponse =
            serde_json::from_str(json).expect("Docker 29 版本响应必须能反序列化");
        assert!(!version.is_podman());
        assert_eq!(version.platform_name(), "linux/amd64");

        let bare: VersionResponse =
            serde_json::from_str(r#"{"Version":"1"}"#).expect("裸版本必须能反序列化");
        assert_eq!(bare.platform_name(), "Docker/Podman");
    }

    #[test]
    fn inspect_tty_flag_defaults_to_false() {
        let inspect: ContainerInspect =
            serde_json::from_str(r#"{"Id":"x","Config":{}}"#).expect("检视必须能反序列化");
        assert!(!inspect.config.expect("配置存在").tty);
    }

    /// 真机回归：Docker 对无标签数据卷返回 `"Labels": null`，端口未映射时
    /// `"Ports": null`；两者都必须收敛成空集合而不是报解析错误。
    #[test]
    fn null_labels_and_ports_deserialize_as_empty() {
        let volume: VolumeItem = serde_json::from_str(
            r#"{"Name":"plain","Driver":"local","Mountpoint":"/var/lib/docker/volumes/plain/_data","CreatedAt":"2026-10-30T23:25:09+08:00","Labels":null,"Options":null,"Scope":"local"}"#,
        )
        .expect("空标签数据卷必须能反序列化");
        assert!(volume.labels.is_empty());
        assert_eq!(volume.into_summary().name, "plain");

        let settings: NetworkSettings =
            serde_json::from_str(r#"{"Ports":null}"#).expect("空端口必须能反序列化");
        assert!(settings.ports.is_empty());
    }
}
