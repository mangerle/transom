//! 容器子命令的 CLI 编排：列表、生命周期、检视、进程与统计。

use super::CliBackend;
use super::parse_human_size;
use crate::error::ContainerError;
use transom_core::{ContainerProcess, ContainerState, ContainerSummary, short_id};

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
        // CLI 的 `stats --format json` 只给汇总值，没有每核/分设备明细。
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
            ..transom_core::ContainerStats::default()
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

/// 解析 docker diff 输出行。
pub(crate) fn parse_diff_output(stdout: &str) -> Vec<transom_core::ContainerFileSystemChange> {
    let mut changes = Vec::new();
    for line in stdout.lines() {
        let line = line.trim();
        if line.len() < 3 {
            continue;
        }
        let (kind_char, path) = line.split_at(1);
        let path = path.trim().to_string();
        let kind = match kind_char {
            "A" => transom_core::ContainerPathChangeKind::Added,
            "D" => transom_core::ContainerPathChangeKind::Deleted,
            _ => transom_core::ContainerPathChangeKind::Modified,
        };
        changes.push(transom_core::ContainerFileSystemChange { path, kind });
    }
    changes
}

impl CliBackend {
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

    pub async fn lifecycle(
        &self,
        container: &str,
        action: transom_core::ContainerLifecycleAction,
    ) -> Result<(), ContainerError> {
        let mut args: Vec<String> = Vec::with_capacity(4);
        match action {
            transom_core::ContainerLifecycleAction::Start => args.push("start".to_string()),
            transom_core::ContainerLifecycleAction::Stop => args.push("stop".to_string()),
            transom_core::ContainerLifecycleAction::Restart => args.push("restart".to_string()),
            transom_core::ContainerLifecycleAction::Pause => args.push("pause".to_string()),
            transom_core::ContainerLifecycleAction::Unpause => args.push("unpause".to_string()),
            transom_core::ContainerLifecycleAction::Kill { signal } => {
                args.push("kill".to_string());
                if let Some(sig) = signal {
                    args.push("-s".to_string());
                    args.push(sig);
                }
            }
            transom_core::ContainerLifecycleAction::Remove { force } => {
                args.push("rm".to_string());
                if force {
                    args.push("-f".to_string());
                }
            }
        }
        args.push(container.to_string());
        self.run_checked(&args).await.map(|_| ())
    }

    pub async fn update_container(
        &self,
        container: &str,
        spec: &transom_core::ContainerUpdateSpec,
    ) -> Result<(), ContainerError> {
        let mut args = vec!["update".to_string()];
        if let Some(mem) = spec.memory_bytes {
            args.push(format!("--memory={mem}"));
        }
        if let Some(cpus) = spec.nano_cpus {
            let cores = cpus as f64 / 1_000_000_000.0;
            args.push(format!("--cpus={cores:.2}"));
        }
        if let Some(restart) = &spec.restart_policy {
            args.push(format!("--restart={restart}"));
        }
        args.push(container.to_string());
        self.run_checked(&args).await.map(|_| ())
    }

    pub async fn commit_container(
        &self,
        container: &str,
        repo: &str,
        tag: Option<&str>,
        comment: Option<&str>,
        author: Option<&str>,
    ) -> Result<String, ContainerError> {
        let mut args = vec!["commit".to_string()];
        if let Some(comment) = comment
            && !comment.is_empty()
        {
            args.push("-m".to_string());
            args.push(comment.to_string());
        }
        if let Some(author) = author
            && !author.is_empty()
        {
            args.push("-a".to_string());
            args.push(author.to_string());
        }
        args.push(container.to_string());
        let reference = match tag {
            Some(t) if !t.is_empty() => format!("{repo}:{t}"),
            _ => repo.to_string(),
        };
        args.push(reference);
        let output = self.run_checked(&args).await?;
        Ok(output.trim().to_string())
    }

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
        self.run_checked(&args).await.map(|_| ())
    }

    pub async fn inspect_container(
        &self,
        container: &str,
    ) -> Result<transom_core::ContainerDetails, ContainerError> {
        let (inspect, raw) = self
            .inspect_value::<crate::models::ContainerInspect>(&[
                "container".to_string(),
                "inspect".to_string(),
                "--size".to_string(),
                container.to_string(),
            ])
            .await?;
        Ok(inspect.into_details(raw))
    }

    pub async fn container_changes(
        &self,
        container: &str,
    ) -> Result<Vec<transom_core::ContainerFileSystemChange>, ContainerError> {
        let stdout = self
            .run_checked(&["diff".to_string(), container.to_string()])
            .await?;
        Ok(parse_diff_output(&stdout))
    }

    pub async fn list_processes(
        &self,
        container: &str,
    ) -> Result<Vec<ContainerProcess>, ContainerError> {
        let stdout = self
            .run_checked(&["top".to_string(), container.to_string(), "aux".to_string()])
            .await?;
        let mut lines = stdout.lines().filter(|line| !line.trim().is_empty());
        let header = lines.next().ok_or_else(|| ContainerError::Unsupported {
            operation: "列出容器进程（容器内 ps 没有输出）".to_string(),
        })?;
        let columns: Vec<&str> = header.split_whitespace().collect();
        let column = |name: &str| {
            columns
                .iter()
                .position(|title| title.eq_ignore_ascii_case(name))
        };
        let pid = column("PID");
        let user = column("USER");
        let cpu = column("%CPU");
        let mem = column("%MEM");
        let command = column("COMMAND")
            .or_else(|| column("CMD"))
            .or_else(|| column("ARGS"));
        let (Some(command_index), Some(_)) = (command, pid) else {
            return Err(ContainerError::Unsupported {
                operation: "列出容器进程（容器内 ps 不支持 aux 列）".to_string(),
            });
        };
        Ok(lines
            .filter_map(|line| {
                let cells: Vec<&str> = line.split_whitespace().collect();
                let cell = |index: Option<usize>| {
                    index
                        .and_then(|index| cells.get(index))
                        .map(|value| value.to_string())
                        .unwrap_or_default()
                };
                Some(ContainerProcess {
                    pid: cell(pid),
                    user: cell(user),
                    cpu_percent: cell(cpu).parse().unwrap_or(0.0),
                    mem_percent: cell(mem).parse().unwrap_or(0.0),
                    command: cells.get(command_index..)?.join(" "),
                })
            })
            .collect())
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_diff_output_lines() {
        let output = "C /etc\nA /etc/app.conf\nD /var/log/test.log\n";
        let changes = parse_diff_output(output);
        assert_eq!(changes.len(), 3);
        assert_eq!(
            changes[0].kind,
            transom_core::ContainerPathChangeKind::Modified
        );
        assert_eq!(changes[0].path, "/etc");
        assert_eq!(
            changes[1].kind,
            transom_core::ContainerPathChangeKind::Added
        );
        assert_eq!(changes[1].path, "/etc/app.conf");
        assert_eq!(
            changes[2].kind,
            transom_core::ContainerPathChangeKind::Deleted
        );
        assert_eq!(changes[2].path, "/var/log/test.log");
    }

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
    fn parses_size_pairs_used_by_cli_stats() {
        assert_eq!(
            parse_size_pair("10MiB / 1GiB"),
            (10 * 1024 * 1024, 1024 * 1024 * 1024)
        );
        assert_eq!(parse_size_pair("1kB / 2kB"), (1000, 2000));
        assert_eq!(parse_size_pair("garbage"), (0, 0));
    }
}
