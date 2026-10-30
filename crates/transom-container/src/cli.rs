//! CLI 兜底后端。
//!
//! 设计原理（Why）：Windows 的 WSL 发行版内 Docker、WSLC 以及无权访问
//! socket 的 Linux 用户，都无法直连 HTTP 端点；`docker`/`podman`/`wslc`
//! 的 CLI 是唯一稳定入口。CLI 后端只实现能力子集（无 resize、无归档 API），
//! 由能力矩阵显式暴露，而不是让用户在点击后才收到失败。
//!
//! 安全约束：所有参数以 argv 数组传递，绝不拼接 Shell 字符串；输出解析
//! 只依赖机器可读的 `--format '{{json .}}'`，不解析人类可读表格。

use crate::discovery::CliSpec;
use crate::error::ContainerError;
use serde::de::DeserializeOwned;
use std::time::Duration;
use tokio::process::Command;
use transom_core::{
    ContainerEngineKind, ContainerImageSummary, ContainerState, ContainerSummary, short_id,
};

/// CLI 单次命令超时；拉取镜像与日志跟随不走这里。
const CLI_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug, Clone)]
pub struct CliBackend {
    pub kind: ContainerEngineKind,
    pub spec: CliSpec,
}

/// 命令输出。
#[derive(Debug, Clone)]
pub struct CliOutput {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

impl CliBackend {
    pub fn new(kind: ContainerEngineKind, spec: CliSpec) -> Self {
        Self { kind, spec }
    }

    /// 构造命令；调用方可继续追加管道与参数。
    pub fn command(&self, args: &[String]) -> Command {
        let mut command = Command::new(&self.spec.program);
        command.args(&self.spec.prefix).args(args);
        command
    }

    fn describe(&self, args: &[String]) -> String {
        self.spec.describe(args)
    }

    /// 执行一条短命令并收集输出。
    ///
    /// # Errors
    /// 子进程启动失败、超时或退出码非零时返回 [`ContainerError::Cli`]；
    /// 需要读取 stderr 的场景由调用方决定是否把 stderr 放进错误。
    pub async fn run(&self, args: &[String]) -> Result<CliOutput, ContainerError> {
        let description = self.describe(args);
        let mut command = self.command(args);
        command.kill_on_drop(true);
        let output = tokio::time::timeout(CLI_TIMEOUT, command.output())
            .await
            .map_err(|_| ContainerError::Cli {
                command: description.clone(),
                message: "命令执行超时".to_string(),
            })?
            .map_err(|error| ContainerError::Cli {
                command: description.clone(),
                message: error.to_string(),
            })?;
        Ok(CliOutput {
            success: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        })
    }

    async fn run_checked(&self, args: &[String]) -> Result<String, ContainerError> {
        let output = self.run(args).await?;
        if !output.success {
            return Err(ContainerError::Cli {
                command: self.describe(args),
                message: output.stderr.trim().to_string(),
            });
        }
        Ok(output.stdout)
    }

    /// 探测 CLI 版本；返回 `(版本, API 版本)`，API 版本拿不到时为空串。
    pub async fn probe(&self) -> Result<(String, String), ContainerError> {
        let format_args = vec![
            "version".to_string(),
            "--format".to_string(),
            "{{.Server.Version}}".to_string(),
        ];
        if let Ok(stdout) = self.run_checked(&format_args).await {
            let version = stdout.trim().to_string();
            if !version.is_empty() {
                return Ok((version, String::new()));
            }
        }
        let output = self.run(&["version".to_string()]).await?;
        if !output.success {
            return Err(ContainerError::Cli {
                command: self.describe(&["version".to_string()]),
                message: output.stderr.trim().to_string(),
            });
        }
        let version = output
            .stdout
            .lines()
            .find_map(|line| {
                line.split_whitespace()
                    .find(|token| token.chars().next().is_some_and(|c| c.is_ascii_digit()))
            })
            .unwrap_or("unknown")
            .to_string();
        Ok((version, String::new()))
    }

    /// 容器列表（含已停止）。
    pub async fn list_containers(&self) -> Result<Vec<ContainerSummary>, ContainerError> {
        let stdout = self
            .run_checked(&[
                "ps".to_string(),
                "-a".to_string(),
                "--format".to_string(),
                "{{json .}}".to_string(),
            ])
            .await?;
        let mut containers = Vec::new();
        for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
            let item: CliContainer =
                serde_json::from_str(line).map_err(|error| ContainerError::Decode {
                    operation: "解析 CLI 容器列表".to_string(),
                    message: format!("{error}，原始行: {line}"),
                })?;
            containers.push(item.into_summary());
        }
        Ok(containers)
    }

    /// 镜像列表。
    pub async fn list_images(&self) -> Result<Vec<ContainerImageSummary>, ContainerError> {
        let stdout = self
            .run_checked(&[
                "images".to_string(),
                "--format".to_string(),
                "{{json .}}".to_string(),
            ])
            .await?;
        let mut images = Vec::new();
        for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
            let item: CliImage =
                serde_json::from_str(line).map_err(|error| ContainerError::Decode {
                    operation: "解析 CLI 镜像列表".to_string(),
                    message: format!("{error}，原始行: {line}"),
                })?;
            images.push(item.into_summary());
        }
        Ok(images)
    }

    /// 执行启停删。
    pub async fn lifecycle(
        &self,
        container: &str,
        action: transom_core::ContainerLifecycleAction,
    ) -> Result<(), ContainerError> {
        let mut args = Vec::with_capacity(3);
        match action {
            transom_core::ContainerLifecycleAction::Start => args.push("start"),
            transom_core::ContainerLifecycleAction::Stop => args.push("stop"),
            transom_core::ContainerLifecycleAction::Restart => args.push("restart"),
            transom_core::ContainerLifecycleAction::Remove { force } => {
                args.push("rm");
                if force {
                    args.push("-f");
                }
            }
        }
        args.push(container);
        let args = args.into_iter().map(str::to_string).collect::<Vec<_>>();
        self.run_checked(&args).await.map(|_| ())
    }

    /// CLI 兜底的重命名：`<程序> rename <容器> <新名>`。
    ///
    /// # Errors
    /// 名称非法或子进程返回非零退出码时转成 [`ContainerError`]。
    pub async fn rename_container(
        &self,
        container: &str,
        name: &str,
    ) -> Result<(), ContainerError> {
        if !transom_core::is_valid_container_name(name) {
            return Err(ContainerError::InvalidName {
                name: name.to_string(),
                reason: "容器名只能包含字母、数字、点、下划线和短横线，且必须以字母或数字开头"
                    .to_string(),
            });
        }
        let args = vec![
            "rename".to_string(),
            container.to_string(),
            name.to_string(),
        ];
        self.run(&args).await.map(|_| ())
    }

    pub async fn pull_image(&self, reference: &str) -> Result<(), ContainerError> {
        self.run_checked(&["pull".to_string(), reference.to_string()])
            .await
            .map(|_| ())
    }

    pub async fn remove_image(&self, image: &str, force: bool) -> Result<(), ContainerError> {
        let mut args = vec!["rmi".to_string()];
        if force {
            args.push("-f".to_string());
        }
        args.push(image.to_string());
        self.run_checked(&args).await.map(|_| ())
    }

    /// `inspect` 系列命令返回数组；取第一条并保留格式化原文。
    async fn inspect_value<T: DeserializeOwned>(
        &self,
        args: &[String],
    ) -> Result<(T, String), ContainerError> {
        let stdout = self.run_checked(args).await?;
        let mut values: Vec<serde_json::Value> =
            serde_json::from_str(&stdout).map_err(|error| ContainerError::Decode {
                operation: "解析 inspect 输出".to_string(),
                message: error.to_string(),
            })?;
        let value = values.pop().ok_or_else(|| ContainerError::Decode {
            operation: "解析 inspect 输出".to_string(),
            message: "返回空数组".to_string(),
        })?;
        let raw = serde_json::to_string_pretty(&value).unwrap_or_default();
        let parsed = serde_json::from_value(value).map_err(|error| ContainerError::Decode {
            operation: "解析 inspect 输出".to_string(),
            message: error.to_string(),
        })?;
        Ok((parsed, raw))
    }

    pub async fn inspect_container(
        &self,
        container: &str,
    ) -> Result<transom_core::ContainerDetails, ContainerError> {
        let (inspect, raw) = self
            .inspect_value::<crate::models::ContainerInspect>(&[
                "container".to_string(),
                "inspect".to_string(),
                container.to_string(),
            ])
            .await?;
        Ok(inspect.into_details(raw))
    }

    pub async fn inspect_image(
        &self,
        image: &str,
    ) -> Result<transom_core::ContainerImageDetails, ContainerError> {
        let (inspect, _raw) = self
            .inspect_value::<crate::models::ImageInspect>(&[
                "image".to_string(),
                "inspect".to_string(),
                image.to_string(),
            ])
            .await?;
        Ok(inspect.into_details())
    }

    pub async fn list_volumes(
        &self,
    ) -> Result<Vec<transom_core::ContainerVolumeSummary>, ContainerError> {
        let stdout = self
            .run_checked(&[
                "volume".to_string(),
                "ls".to_string(),
                "--format".to_string(),
                "{{json .}}".to_string(),
            ])
            .await?;
        let mut volumes = Vec::new();
        for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
            let item: crate::models::VolumeItem =
                serde_json::from_str(line).map_err(|error| ContainerError::Decode {
                    operation: "解析 CLI 数据卷列表".to_string(),
                    message: format!("{error}，原始行: {line}"),
                })?;
            volumes.push(item.into_summary());
        }
        Ok(volumes)
    }

    pub async fn inspect_volume(
        &self,
        name: &str,
    ) -> Result<transom_core::ContainerVolumeDetails, ContainerError> {
        let (item, _raw) = self
            .inspect_value::<crate::models::VolumeItem>(&[
                "volume".to_string(),
                "inspect".to_string(),
                name.to_string(),
            ])
            .await?;
        Ok(transom_core::ContainerVolumeDetails {
            summary: item.into_summary(),
            // CLI 列表不携带挂载信息，占用容器留给 HTTP 后端展示。
            used_by: Vec::new(),
        })
    }

    pub async fn create_volume(
        &self,
        name: &str,
        driver: Option<&str>,
    ) -> Result<(), ContainerError> {
        let mut args = vec!["volume".to_string(), "create".to_string()];
        if let Some(driver) = driver {
            args.push("--driver".to_string());
            args.push(driver.to_string());
        }
        args.push(name.to_string());
        self.run_checked(&args).await.map(|_| ())
    }

    pub async fn remove_volume(&self, name: &str, force: bool) -> Result<(), ContainerError> {
        let mut args = vec!["volume".to_string(), "rm".to_string()];
        if force {
            args.push("-f".to_string());
        }
        args.push(name.to_string());
        self.run_checked(&args).await.map(|_| ())
    }

    pub async fn list_networks(
        &self,
    ) -> Result<Vec<transom_core::ContainerNetworkSummary>, ContainerError> {
        let stdout = self
            .run_checked(&[
                "network".to_string(),
                "ls".to_string(),
                "--format".to_string(),
                "{{json .}}".to_string(),
            ])
            .await?;
        let mut networks = Vec::new();
        for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
            let item: crate::models::NetworkItem =
                serde_json::from_str(line).map_err(|error| ContainerError::Decode {
                    operation: "解析 CLI 网络列表".to_string(),
                    message: format!("{error}，原始行: {line}"),
                })?;
            networks.push(item.summary());
        }
        Ok(networks)
    }

    pub async fn inspect_network(
        &self,
        network: &str,
    ) -> Result<transom_core::ContainerNetworkDetails, ContainerError> {
        let (item, _raw) = self
            .inspect_value::<crate::models::NetworkItem>(&[
                "network".to_string(),
                "inspect".to_string(),
                network.to_string(),
            ])
            .await?;
        Ok(item.into_details())
    }

    pub async fn create_network(
        &self,
        name: &str,
        driver: Option<&str>,
    ) -> Result<(), ContainerError> {
        let mut args = vec!["network".to_string(), "create".to_string()];
        if let Some(driver) = driver {
            args.push("--driver".to_string());
            args.push(driver.to_string());
        }
        args.push(name.to_string());
        self.run_checked(&args).await.map(|_| ())
    }

    pub async fn remove_network(&self, network: &str) -> Result<(), ContainerError> {
        self.run_checked(&["network".to_string(), "rm".to_string(), network.to_string()])
            .await
            .map(|_| ())
    }

    /// CLI 统计是单次快照；解析人类可读字段。
    pub async fn container_stats(
        &self,
        container: &str,
    ) -> Result<transom_core::ContainerStats, ContainerError> {
        let stdout = self
            .run_checked(&[
                "stats".to_string(),
                "--no-stream".to_string(),
                "--format".to_string(),
                "{{json .}}".to_string(),
                container.to_string(),
            ])
            .await?;
        let line = stdout
            .lines()
            .find(|line| !line.trim().is_empty())
            .ok_or_else(|| ContainerError::Decode {
                operation: "解析 CLI 统计".to_string(),
                message: "没有输出".to_string(),
            })?;
        let stats: CliStats =
            serde_json::from_str(line).map_err(|error| ContainerError::Decode {
                operation: "解析 CLI 统计".to_string(),
                message: format!("{error}，原始行: {line}"),
            })?;
        Ok(stats.into_stats())
    }
}

/// `docker ps --format '{{json .}}'` 的字段。
#[derive(Debug, serde::Deserialize)]
struct CliContainer {
    #[serde(rename = "ID")]
    id: String,
    #[serde(rename = "Names", default)]
    names: String,
    #[serde(rename = "Image", default)]
    image: String,
    #[serde(rename = "State", default)]
    state: String,
    #[serde(rename = "Status", default)]
    status: String,
    #[serde(rename = "Ports", default)]
    ports: String,
    /// 形如 `k=v,k2=v2`；用于识别 Compose 项目。
    #[serde(rename = "Labels", default)]
    labels: String,
}

impl CliContainer {
    fn into_summary(self) -> ContainerSummary {
        let name = self
            .names
            .split(',')
            .next()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| short_id(&self.id));
        let labels = parse_labels(&self.labels);
        let compose_project = labels
            .get("com.docker.compose.project")
            .cloned()
            .filter(|value| !value.is_empty());
        let compose_working_dir = labels
            .get("com.docker.compose.project.working_dir")
            .cloned()
            .filter(|value| !value.is_empty());
        let compose_config_files = labels
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
            created: 0,
            ports: parse_cli_ports(&self.ports),
            compose_project,
            compose_working_dir,
            compose_config_files,
            // CLI 的 `ps` 输出没有挂载明细，留空；界面上的「仅未使用卷」筛选在
            // CLI 后端会被隐藏，避免把在用卷误判成空闲。
            mounts: Vec::new(),
        }
    }
}

fn parse_labels(value: &str) -> std::collections::HashMap<String, String> {
    value
        .split(',')
        .filter_map(|pair| pair.split_once('='))
        .map(|(key, value)| (key.trim().to_string(), value.trim().to_string()))
        .collect()
}

/// `docker images --format '{{json .}}'` 的字段。
#[derive(Debug, serde::Deserialize)]
struct CliImage {
    #[serde(rename = "ID", default)]
    id: String,
    #[serde(rename = "Repository", default)]
    repository: String,
    #[serde(rename = "Tag", default)]
    tag: String,
    #[serde(rename = "Size", default)]
    size: String,
}

impl CliImage {
    fn into_summary(self) -> ContainerImageSummary {
        let mut tags = Vec::with_capacity(1);
        if !self.repository.is_empty() && self.repository != "<none>" {
            tags.push(format!("{}:{}", self.repository, self.tag));
        }
        ContainerImageSummary {
            id: self.id,
            tags,
            size: parse_human_size(&self.size).unwrap_or(0),
            created: 0,
        }
    }
}

/// 解析 `0.0.0.0:8080->80/tcp, :::8080->80/tcp` 这类端口摘要。
fn parse_cli_ports(value: &str) -> Vec<transom_core::ContainerPort> {
    let mut ports = Vec::new();
    for entry in value
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
    {
        let (public, private) = match entry.split_once("->") {
            Some((public, private)) => (Some(public), private),
            None => (None, entry),
        };
        let (private_port, protocol) = match private.split_once('/') {
            Some((port, protocol)) => (port, protocol),
            None => (private, "tcp"),
        };
        let Ok(private_port) = private_port.parse::<u16>() else {
            continue;
        };
        let public_port = public.and_then(|public| {
            public
                .rsplit(':')
                .next()
                .and_then(|port| port.parse::<u16>().ok())
        });
        ports.push(transom_core::ContainerPort {
            private_port,
            public_port,
            ip: None,
            protocol: protocol.to_string(),
        });
    }
    ports
}

/// 解析 `123MB` / `1.2GB` / `512kB` 这类人类可读体积。
fn parse_human_size(value: &str) -> Option<u64> {
    let value = value.trim();
    let split = value
        .find(|c: char| !c.is_ascii_digit() && c != '.')
        .unwrap_or(value.len());
    let number: f64 = value[..split].parse().ok()?;
    let unit = value[split..].trim().to_ascii_uppercase();
    let multiplier = match unit.as_str() {
        "" | "B" => 1.0,
        "KB" => 1_000.0,
        "MB" => 1_000_000.0,
        "GB" => 1_000_000_000.0,
        "TB" => 1_000_000_000_000.0,
        "KIB" => 1024.0,
        "MIB" => 1024.0 * 1024.0,
        "GIB" => 1024.0 * 1024.0 * 1024.0,
        "TIB" => 1024.0 * 1024.0 * 1024.0 * 1024.0,
        _ => return None,
    };
    Some((number * multiplier) as u64)
}

/// 解析 `docker stats --format '{{json .}}'` 的字段。
#[derive(Debug, serde::Deserialize, Default)]
struct CliStats {
    #[serde(rename = "CPUPerc", default)]
    cpu_percent: String,
    #[serde(rename = "MemUsage", default)]
    memory: String,
    #[serde(rename = "MemPerc", default)]
    memory_percent: String,
    #[serde(rename = "NetIO", default)]
    network: String,
    #[serde(rename = "BlockIO", default)]
    block: String,
    #[serde(rename = "PIDs", default)]
    pids: String,
}

impl CliStats {
    fn into_stats(self) -> transom_core::ContainerStats {
        let (memory_used, memory_limit) = parse_size_pair(&self.memory);
        let (network_rx, network_tx) = parse_size_pair(&self.network);
        let (block_read, block_write) = parse_size_pair(&self.block);
        transom_core::ContainerStats {
            cpu_percent: parse_percent(&self.cpu_percent),
            memory_used,
            memory_limit,
            memory_percent: parse_percent(&self.memory_percent),
            network_rx,
            network_tx,
            block_read,
            block_write,
            pids: self.pids.trim().parse().unwrap_or(0),
        }
    }
}

fn parse_percent(value: &str) -> f32 {
    value
        .trim()
        .trim_end_matches('%')
        .trim()
        .parse()
        .unwrap_or(0.0)
}

/// 解析 `10MiB / 1GiB`、`1kB / 2kB` 形式的成对体积。
fn parse_size_pair(value: &str) -> (u64, u64) {
    let mut parts = value.split('/');
    let first = parts.next().and_then(parse_human_size).unwrap_or(0);
    let second = parts.next().and_then(parse_human_size).unwrap_or(0);
    (first, second)
}

#[cfg(test)]
mod tests {
    use super::{parse_cli_ports, parse_human_size, parse_size_pair};

    #[test]
    fn parses_port_summaries() {
        let ports = parse_cli_ports("0.0.0.0:8080->80/tcp, :::8443->443/tcp");
        assert_eq!(ports.len(), 2);
        assert_eq!(ports[0].private_port, 80);
        assert_eq!(ports[0].public_port, Some(8080));
        assert_eq!(ports[1].protocol, "tcp");
        assert!(parse_cli_ports("").is_empty());
    }

    #[test]
    fn parses_human_sizes() {
        assert_eq!(parse_human_size("512B"), Some(512));
        assert_eq!(parse_human_size("1.5kB"), Some(1500));
        assert_eq!(parse_human_size("2MB"), Some(2_000_000));
        assert_eq!(parse_human_size("1GiB"), Some(1_073_741_824));
        assert_eq!(parse_human_size("unknown"), None);
    }

    #[test]
    fn parses_size_pairs_used_by_cli_stats() {
        assert_eq!(
            parse_size_pair("10MiB / 1GiB"),
            (10 * 1024 * 1024, 1024 * 1024 * 1024)
        );
        assert_eq!(parse_size_pair("1kB / 2kB"), (1000, 2000));
        assert_eq!(parse_size_pair("garbage"), (0, 0));
    }
}
