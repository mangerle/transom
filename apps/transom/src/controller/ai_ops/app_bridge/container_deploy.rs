//! 容器一键创建启动（`create_and_start_container`）与 Compose YAML 直接部署（`deploy_compose_yaml`）。
//! 支持本机（`target` 省略或 `"local"`）与远端服务器（自动连接目标 SSH 会话后执行）。

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use serde_json::Value;
use transom_ai::{RemoteExecutor, RemoteFsExecutor, ToolError};
use transom_container::{ContainerError, Engine, discover_all};
use transom_core::{ContainerCreateSpec, ContainerEngineKind, ContainerPortPublish};

use super::ControllerAppActionBridge;
use super::target_resolver::{
    ensure_remote_dir, ensure_sftp, guard_readonly, resolve_or_connect_target, shell_quote,
};
use crate::controller::ai_ops::bridge::{SshRemoteExecutor, SshRemoteFsBridge};

/// 一键创建并启动容器（支持本机引擎与远端 SSH 主机）。
pub(super) async fn create_and_start_container(
    bridge: &ControllerAppActionBridge,
    args: &Value,
) -> Result<String, ToolError> {
    let image = req_str(args, "image")?;
    let pull = args["pull"].as_bool().unwrap_or(false);
    let mut spec = build_create_spec(image, args)?;
    spec.validate()
        .map_err(|reason| ToolError::InvalidArguments {
            tool: "container_workspace",
            reason,
        })?;

    if let Some(target) = remote_target(args) {
        let session = resolve_or_connect_target(bridge, Some(target), 30).await?;
        guard_readonly(&session, true)?;
        return create_remote_container(bridge, &session, &spec, pull).await;
    }

    let engine = pick_local_engine(args["engine_id"].as_str()).await?;
    if pull {
        engine
            .pull_image(&spec.image, &mut |_, _| {})
            .await
            .map_err(map_err)?;
    }

    match engine.create_container(&spec).await {
        Ok(cid) => Ok(format!(
            "已在本机创建并启动容器: {}（镜像: {}）",
            cid, spec.image
        )),
        Err(ContainerError::Unsupported { .. }) => {
            create_local_via_cli(engine.info.engine.kind, &spec).await
        }
        Err(err) => Err(map_err(err)),
    }
}

/// 传入 Compose YAML 文本直接落盘并执行 `up -d`（支持本机与远端）。
pub(super) async fn deploy_compose_yaml(
    bridge: &ControllerAppActionBridge,
    args: &Value,
) -> Result<String, ToolError> {
    let yaml = args["yaml"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| ToolError::InvalidArguments {
            tool: "container_workspace",
            reason: "deploy_compose_yaml 必须提供非空 yaml 内容".to_owned(),
        })?;
    let pull = args["pull"].as_bool().unwrap_or(false);
    let safe_name = sanitize_project_name(args["name"].as_str().unwrap_or("app"));

    if let Some(target) = remote_target(args) {
        let session = resolve_or_connect_target(bridge, Some(target), 30).await?;
        guard_readonly(&session, true)?;
        let dir = match args["project_dir"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(d) => d.to_owned(),
            None => {
                let home = session
                    .remote
                    .exec_command_capture("printf '%s' \"$HOME\"")
                    .await
                    .map_err(|e| ToolError::ExecutionFailed {
                        tool: "container_workspace",
                        reason: e.to_string(),
                    })?;
                let home_dir = home.stdout.trim();
                let base = if home_dir.is_empty() {
                    "/tmp"
                } else {
                    home_dir
                };
                format!("{base}/transom-compose/{safe_name}")
            }
        };
        ensure_remote_dir(&session.remote, &dir).await?;
        let compose_file = format!("{}/docker-compose.yml", dir.trim_end_matches('/'));
        let fs = SshRemoteFsBridge {
            remote: session.remote.clone(),
            sftp: Some(ensure_sftp(&session).await?),
        };
        fs.write_file(&compose_file, yaml, true).await?;

        let pull_flag = if pull { " --pull always" } else { "" };
        let qdir = shell_quote(&dir);
        let cmd = format!(
            "cd {qdir} && (docker compose up -d{pull_flag} || docker-compose up -d{pull_flag})"
        );
        let exec = SshRemoteExecutor {
            remote: session.remote.clone(),
            tasks: Some(bridge.deps.tasks.clone()),
            host_label: session.meta.host_name.clone(),
        };
        let out = exec
            .exec(&cmd, Duration::from_secs(180), bridge.deps.cancel.clone())
            .await?;
        let text = format!(
            "远端主机: {}\n项目目录: {dir}\n{}\n{}",
            session.meta.host_name, out.stdout, out.stderr
        );
        return if out.exit_code == 0 {
            Ok(text)
        } else {
            Err(ToolError::ExecutionFailed {
                tool: "container_workspace",
                reason: text,
            })
        };
    }

    let dir = match args["project_dir"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(d) => PathBuf::from(d),
        None => {
            let base = std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .map(PathBuf::from)
                .unwrap_or_else(std::env::temp_dir);
            base.join("transom-compose").join(&safe_name)
        }
    };
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| ToolError::ExecutionFailed {
            tool: "container_workspace",
            reason: format!("创建本机 Compose 目录失败: {e}"),
        })?;
    let compose_file = dir.join("docker-compose.yml");
    tokio::fs::write(&compose_file, yaml)
        .await
        .map_err(|e| ToolError::ExecutionFailed {
            tool: "container_workspace",
            reason: format!("写入 docker-compose.yml 失败: {e}"),
        })?;
    run_local_compose_action(&dir, "compose_up", pull).await
}

/// 在本机或远端执行 `compose_up` / `compose_down`。
pub(super) async fn handle_compose_lifecycle(
    bridge: &ControllerAppActionBridge,
    action: &str,
    args: &Value,
) -> Result<String, ToolError> {
    let dir = req_str(args, "project_dir")?;
    if let Some(target) = remote_target(args) {
        let session = resolve_or_connect_target(bridge, Some(target), 30).await?;
        guard_readonly(&session, true)?;
        let qdir = shell_quote(dir);
        let sub = if action == "compose_up" {
            "up -d"
        } else {
            "down --remove-orphans"
        };
        let cmd = format!("cd {qdir} && (docker compose {sub} || docker-compose {sub})");
        let out = session
            .remote
            .exec_command_capture(&cmd)
            .await
            .map_err(|e| ToolError::ExecutionFailed {
                tool: "container_workspace",
                reason: e.to_string(),
            })?;
        let text = format!("{}\n{}", out.stdout, out.stderr);
        return if out.exit_status == Some(0) {
            Ok(text)
        } else {
            Err(ToolError::ExecutionFailed {
                tool: "container_workspace",
                reason: text,
            })
        };
    }
    run_local_compose_action(&PathBuf::from(dir), action, false).await
}

async fn run_local_compose_action(
    dir: &PathBuf,
    action: &str,
    pull: bool,
) -> Result<String, ToolError> {
    let mut cmd = tokio::process::Command::new("docker");
    cmd.arg("compose");
    if action == "compose_up" {
        cmd.arg("up").arg("-d");
        if pull {
            cmd.arg("--pull").arg("always");
        }
    } else {
        cmd.arg("down").arg("--remove-orphans");
    }
    cmd.current_dir(dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let child = cmd.spawn().map_err(|e| ToolError::ExecutionFailed {
        tool: "container_workspace",
        reason: e.to_string(),
    })?;
    let out = tokio::time::timeout(Duration::from_secs(180), child.wait_with_output())
        .await
        .map_err(|_| ToolError::ExecutionTimeout { timeout_secs: 180 })?
        .map_err(|e| ToolError::ExecutionFailed {
            tool: "container_workspace",
            reason: e.to_string(),
        })?;
    let text = format!(
        "目录: {}\n{}\n{}",
        dir.display(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    if out.status.success() {
        Ok(text)
    } else {
        Err(ToolError::ExecutionFailed {
            tool: "container_workspace",
            reason: text,
        })
    }
}

async fn create_local_via_cli(
    kind: ContainerEngineKind,
    spec: &ContainerCreateSpec,
) -> Result<String, ToolError> {
    let bin = match kind {
        ContainerEngineKind::Podman => "podman",
        _ => "docker",
    };
    let mut args = build_run_args(spec);
    let mut cmd = tokio::process::Command::new(bin);
    cmd.arg("run").arg("-d");
    for a in args.drain(..) {
        cmd.arg(a);
    }
    cmd.stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let child = cmd.spawn().map_err(|e| ToolError::ExecutionFailed {
        tool: "container_workspace",
        reason: e.to_string(),
    })?;
    let out = tokio::time::timeout(Duration::from_secs(120), child.wait_with_output())
        .await
        .map_err(|_| ToolError::ExecutionTimeout { timeout_secs: 120 })?
        .map_err(|e| ToolError::ExecutionFailed {
            tool: "container_workspace",
            reason: e.to_string(),
        })?;
    if out.status.success() {
        let cid = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        Ok(format!(
            "已在本机创建并启动容器: {cid}（镜像: {}）",
            spec.image
        ))
    } else {
        Err(ToolError::ExecutionFailed {
            tool: "container_workspace",
            reason: String::from_utf8_lossy(&out.stderr).to_string(),
        })
    }
}

async fn create_remote_container(
    bridge: &ControllerAppActionBridge,
    session: &crate::controller::ai_ops::mcp_bridge::McpLiveSession,
    spec: &ContainerCreateSpec,
    pull: bool,
) -> Result<String, ToolError> {
    let args = build_run_args(spec);
    let quoted_args: Vec<String> = args.iter().map(|a| shell_quote(a)).collect();
    let qimg = shell_quote(&spec.image);
    let pull_prefix = if pull {
        format!("$BIN pull {qimg} && ")
    } else {
        String::new()
    };
    let cmd = format!(
        "BIN=$(command -v docker || command -v podman); \
         [ -n \"$BIN\" ] || {{ echo '未检测到 docker 或 podman' >&2; exit 127; }}; \
         {pull_prefix}$BIN run -d {}",
        quoted_args.join(" ")
    );
    let exec = SshRemoteExecutor {
        remote: session.remote.clone(),
        tasks: Some(bridge.deps.tasks.clone()),
        host_label: session.meta.host_name.clone(),
    };
    let out = exec
        .exec(&cmd, Duration::from_secs(180), bridge.deps.cancel.clone())
        .await?;
    if out.exit_code == 0 {
        let cid = out.stdout.trim();
        Ok(format!(
            "已在远端「{}」创建并启动容器: {cid}（镜像: {}）",
            session.meta.host_name, spec.image
        ))
    } else {
        Err(ToolError::ExecutionFailed {
            tool: "container_workspace",
            reason: format!("{}\n{}", out.stdout, out.stderr),
        })
    }
}

fn build_run_args(spec: &ContainerCreateSpec) -> Vec<String> {
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

fn build_create_spec(image: &str, args: &Value) -> Result<ContainerCreateSpec, ToolError> {
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

fn sanitize_project_name(raw: &str) -> String {
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

fn remote_target(args: &Value) -> Option<&str> {
    args["target"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("local"))
}

async fn pick_local_engine(wanted: Option<&str>) -> Result<Engine, ToolError> {
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

fn req_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, ToolError> {
    args[key]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ToolError::InvalidArguments {
            tool: "container_workspace",
            reason: format!("缺少必要参数 {key}"),
        })
}

fn str_array(args: &Value, key: &str) -> Vec<String> {
    args[key]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

fn map_err(err: ContainerError) -> ToolError {
    ToolError::ExecutionFailed {
        tool: "container_workspace",
        reason: err.to_string(),
    }
}
