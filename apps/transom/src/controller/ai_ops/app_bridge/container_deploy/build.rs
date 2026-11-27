//! 容器创建参数解析与引擎选择辅助。

use serde_json::Value;
use transom_ai::ToolError;
use transom_container::{ContainerError, Engine, discover_all};
use transom_core::{ContainerCreateSpec, ContainerPortPublish};

pub(super) fn build_run_args(spec: &ContainerCreateSpec) -> Vec<String> {
    let mut argv = Vec::new();
    if !spec.name.is_empty() {
        argv.push("--name".to_owned());
        argv.push(spec.name.clone());
    }
    if let Some(restart) = &spec.restart {
        argv.push("--restart".to_owned());
        argv.push(restart.clone());
    }
    if let Some(net) = &spec.network_mode {
        argv.push("--network".to_owned());
        argv.push(net.clone());
    }
    for p in &spec.ports {
        argv.push("-p".to_owned());
        argv.push(format!("{}:{}/{}", p.host, p.container, p.protocol));
    }
    for e in &spec.env {
        argv.push("-e".to_owned());
        argv.push(e.clone());
    }
    for v in &spec.binds {
        argv.push("-v".to_owned());
        argv.push(v.clone());
    }
    argv.push(spec.image.clone());
    for c in &spec.command {
        argv.push(c.clone());
    }
    argv
}

pub(super) fn build_create_spec(
    image: &str,
    args: &Value,
) -> Result<ContainerCreateSpec, ToolError> {
    let mut ports = Vec::new();
    if let Some(arr) = args["ports"].as_array() {
        for item in arr {
            if let Some(raw) = item.as_str() {
                ports.push(parse_port_publish(raw)?);
            } else if let (Some(h), Some(c)) = (item["host"].as_u64(), item["container"].as_u64()) {
                ports.push(ContainerPortPublish {
                    host: h as u16,
                    container: c as u16,
                    protocol: item["protocol"].as_str().unwrap_or("tcp").to_owned(),
                });
            }
        }
    }
    Ok(ContainerCreateSpec {
        name: args["name"].as_str().unwrap_or("").trim().to_owned(),
        image: image.to_owned(),
        command: str_array(args, "command"),
        env: str_array(args, "env"),
        ports,
        binds: if args["volumes"].is_array() {
            str_array(args, "volumes")
        } else {
            str_array(args, "binds")
        },
        restart: args["restart"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned),
        network_mode: args["network"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned),
        ..Default::default()
    })
}

fn parse_port_publish(raw: &str) -> Result<ContainerPortPublish, ToolError> {
    let trimmed = raw.trim();
    let (pair, protocol) = match trimmed.split_once('/') {
        Some((p, proto)) => (p, proto.to_ascii_lowercase()),
        None => (trimmed, "tcp".to_owned()),
    };
    let (h, c) = pair
        .split_once(':')
        .ok_or_else(|| ToolError::InvalidArguments {
            tool: "container_workspace",
            reason: format!("端口映射格式应为 host:container[/tcp|udp]，收到: {trimmed}"),
        })?;
    let host = h
        .trim()
        .parse::<u16>()
        .map_err(|_| ToolError::InvalidArguments {
            tool: "container_workspace",
            reason: format!("无效宿主端口: {h}"),
        })?;
    let container = c
        .trim()
        .parse::<u16>()
        .map_err(|_| ToolError::InvalidArguments {
            tool: "container_workspace",
            reason: format!("无效容器端口: {c}"),
        })?;
    Ok(ContainerPortPublish {
        host,
        container,
        protocol,
    })
}

pub(super) fn sanitize_project_name(raw: &str) -> String {
    let cleaned: String = raw
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches('-');
    if trimmed.is_empty() {
        "app".to_owned()
    } else {
        trimmed.to_owned()
    }
}

pub(super) fn remote_target(args: &Value) -> Option<&str> {
    args["target"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("local"))
}

pub(super) async fn pick_local_engine(wanted: Option<&str>) -> Result<Engine, ToolError> {
    let (engines, failures) = discover_all().await;
    if let Some(id) = wanted.filter(|s| !s.is_empty())
        && let Some(found) = engines.into_iter().find(|e| {
            e.info.engine.endpoint.describe().contains(id)
                || e.info.engine.kind.label().eq_ignore_ascii_case(id)
        })
    {
        return Ok(found);
    }
    let (engines_retry, _) = discover_all().await;
    engines_retry
        .into_iter()
        .next()
        .ok_or_else(|| ToolError::ExecutionFailed {
            tool: "container_workspace",
            reason: if failures.is_empty() {
                "本机未发现可用的 Docker 或 Podman 引擎".to_owned()
            } else {
                failures.join("；")
            },
        })
}

pub(super) fn req_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, ToolError> {
    args[key]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ToolError::InvalidArguments {
            tool: "container_workspace",
            reason: format!("缺少必要参数 {key}"),
        })
}

pub(super) fn str_array(args: &Value, key: &str) -> Vec<String> {
    args[key]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn map_err(err: ContainerError) -> ToolError {
    ToolError::ExecutionFailed {
        tool: "container_workspace",
        reason: err.to_string(),
    }
}
