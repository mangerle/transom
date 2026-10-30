//! Docker/Podman REST API 的线格式结构体与到领域模型的转换。
//!
//! 原则（Why）：外部 JSON 只在本模块出现一次，立即收敛为
//! `transom-core` 的强类型；引擎字段增删或大小写差异不会扩散到控制器与界面。

use serde::Deserialize;
use serde::de::Deserializer;
use std::collections::HashMap;
use transom_core::{
    ContainerDetails, ContainerImageDetails, ContainerImageSummary, ContainerMount,
    ContainerNetworkAttachment, ContainerNetworkDetails, ContainerNetworkSummary, ContainerPort,
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
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct InspectState {
    #[serde(rename = "Status", default)]
    pub status: Option<String>,
    #[serde(rename = "ExitCode", default)]
    pub exit_code: Option<i64>,
    #[serde(rename = "StartedAt", default)]
    pub started_at: Option<String>,
    #[serde(rename = "FinishedAt", default)]
    pub finished_at: Option<String>,
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
            status: state.status.unwrap_or_default(),
            created: parse_rfc3339_secs(self.created.as_deref()).unwrap_or(0),
            started_at: state.started_at,
            finished_at: state.finished_at,
            exit_code: state.exit_code,
            restart_policy,
            ports,
            mounts: self.mounts.into_iter().map(MountItem::into_mount).collect(),
            compose_project: config.labels.get("com.docker.compose.project").cloned(),
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
}

impl VolumeItem {
    pub fn into_summary(self) -> ContainerVolumeSummary {
        let mut labels: Vec<(String, String)> = self.labels.into_iter().collect();
        labels.sort_by(|left, right| left.0.cmp(&right.0));
        ContainerVolumeSummary {
            name: self.name,
            driver: self.driver,
            mountpoint: self.mountpoint,
            created: self.created_at,
            labels,
        }
    }
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
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Ipam {
    #[serde(rename = "Config", default)]
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

    pub fn into_details(self) -> ContainerNetworkDetails {
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
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct RootFs {
    #[serde(rename = "Layers", default)]
    pub layers: Vec<String>,
}

impl ImageInspect {
    pub fn into_details(self) -> ContainerImageDetails {
        ContainerImageDetails {
            id: self.id,
            tags: self.repo_tags.unwrap_or_default(),
            size: self.size,
            created: parse_rfc3339_secs(self.created.as_deref()).unwrap_or(0),
            os: self.os,
            architecture: self.architecture,
            layers: self.root_fs.map(|root| root.layers.len()).unwrap_or(0),
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
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct PidsStats {
    #[serde(rename = "current", default)]
    pub current: u64,
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
        let memory_used = self.memory_stats.usage;
        let memory_limit = self.memory_stats.limit;
        let memory_percent = if memory_limit > 0 {
            (memory_used as f64 / memory_limit as f64 * 100.0) as f32
        } else {
            0.0
        };
        let (mut network_rx, mut network_tx) = (0u64, 0u64);
        if let Some(networks) = &self.networks {
            for interface in networks.values() {
                network_rx = network_rx.saturating_add(interface.rx_bytes);
                network_tx = network_tx.saturating_add(interface.tx_bytes);
            }
        }
        let (mut block_read, mut block_write) = (0u64, 0u64);
        if let Some(entries) = self
            .blkio_stats
            .as_ref()
            .and_then(|stats| stats.io_service_bytes_recursive.as_ref())
        {
            for entry in entries {
                match entry.op.as_str() {
                    "read" | "Read" => block_read = block_read.saturating_add(entry.value),
                    "write" | "Write" => block_write = block_write.saturating_add(entry.value),
                    _ => {}
                }
            }
        }
        ContainerStats {
            cpu_percent,
            memory_used,
            memory_limit,
            memory_percent,
            network_rx,
            network_tx,
            block_read,
            block_write,
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
        ContainerInspect, ContainerListItem, ImageListItem, NetworkSettings, VersionResponse,
        VolumeItem,
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
