//! 领域 2（工作区/标签/面板）、领域 4（监控/进程）与领域 5（端口转发）。

use serde_json::{Value, json};
use transom_ai::ToolError;

use super::ControllerAppActionBridge;

pub(super) async fn handle_workspace(
    bridge: &ControllerAppActionBridge,
    action: &str,
    args: &Value,
) -> Result<String, ToolError> {
    bridge
        .request_ui_action("workspace_control", action, args)
        .await
}

pub(super) async fn handle_forward(
    bridge: &ControllerAppActionBridge,
    action: &str,
    args: &Value,
) -> Result<String, ToolError> {
    bridge
        .request_ui_action("port_forward_ops", action, args)
        .await
}

pub(super) async fn handle_monitor(
    bridge: &ControllerAppActionBridge,
    action: &str,
    args: &Value,
) -> Result<String, ToolError> {
    match action {
        "toggle_sampling" => {
            bridge
                .request_ui_action("host_monitor", "toggle_sampling", args)
                .await
        }
        "snapshot" => monitor_snapshot(bridge).await,
        "list_processes" => list_processes(bridge).await,
        "kill_process" => kill_process(bridge, args).await,
        "probe_host" => probe_host(args).await,
        other => Err(ToolError::InvalidArguments {
            tool: "host_monitor",
            reason: format!("未知监控动作: {other}"),
        }),
    }
}

async fn monitor_snapshot(bridge: &ControllerAppActionBridge) -> Result<String, ToolError> {
    if let Some(remote) = &bridge.deps.target.active_remote {
        let out = remote
            .exec_command_capture(
                "uname -a; uptime; free -m; df -hT -x tmpfs -x devtmpfs; ss -tulnp 2>/dev/null | head -n 30",
            )
            .await
            .map_err(|e| ToolError::ExecutionFailed {
                tool: "host_monitor",
                reason: e.to_string(),
            })?;
        return Ok(out.stdout);
    }

    let sampler = bridge.deps.runtime.local_processes.clone();
    tokio::task::spawn_blocking(move || {
        let mut guard = sampler.lock();
        let procs = guard.sample();
        let top: Vec<_> = procs
            .iter()
            .take(15)
            .map(|p| {
                json!({
                    "pid": p.pid,
                    "cmdline": p.cmdline,
                    "cpu_percent": p.cpu_percent,
                    "rss_bytes": p.rss_bytes,
                })
            })
            .collect();
        serde_json::to_string_pretty(&json!({
            "target": "local",
            "process_count": procs.len(),
            "top_processes": top,
        }))
        .unwrap_or_default()
    })
    .await
    .map_err(|e| ToolError::ExecutionFailed {
        tool: "host_monitor",
        reason: e.to_string(),
    })
}

async fn list_processes(bridge: &ControllerAppActionBridge) -> Result<String, ToolError> {
    if let Some(remote) = &bridge.deps.target.active_remote {
        let out = remote
            .exec_command_capture("ps -eo pid,ppid,user,%cpu,%mem,comm --sort=-%cpu | head -n 60")
            .await
            .map_err(|e| ToolError::ExecutionFailed {
                tool: "host_monitor",
                reason: e.to_string(),
            })?;
        return Ok(out.stdout);
    }

    let sampler = bridge.deps.runtime.local_processes.clone();
    tokio::task::spawn_blocking(move || {
        let mut guard = sampler.lock();
        let mut procs = guard.sample();
        procs.sort_by(|a, b| {
            b.cpu_percent
                .partial_cmp(&a.cpu_percent)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let rows: Vec<_> = procs
            .into_iter()
            .take(60)
            .map(|p| {
                json!({
                    "pid": p.pid,
                    "user": p.user,
                    "cpu_percent": p.cpu_percent,
                    "rss_bytes": p.rss_bytes,
                    "cmdline": p.cmdline,
                    "args": p.args,
                })
            })
            .collect();
        serde_json::to_string_pretty(&rows).unwrap_or_else(|_| "[]".to_owned())
    })
    .await
    .map_err(|e| ToolError::ExecutionFailed {
        tool: "host_monitor",
        reason: e.to_string(),
    })
}

async fn kill_process(
    bridge: &ControllerAppActionBridge,
    args: &Value,
) -> Result<String, ToolError> {
    let pid = args["pid"]
        .as_u64()
        .ok_or_else(|| ToolError::InvalidArguments {
            tool: "host_monitor",
            reason: "kill_process 必须提供整数 pid".to_owned(),
        })?;
    let signal = match args["signal"].as_str().unwrap_or("TERM") {
        "KILL" | "9" => "9",
        _ => "15",
    };

    if let Some(remote) = &bridge.deps.target.active_remote {
        let out = remote
            .exec_command_capture(&format!("kill -{signal} {pid}"))
            .await
            .map_err(|e| ToolError::ExecutionFailed {
                tool: "host_monitor",
                reason: e.to_string(),
            })?;
        if out.exit_status == Some(0) {
            return Ok(format!("已向远端进程 {pid} 发送信号 -{signal}"));
        }
        return Err(ToolError::ExecutionFailed {
            tool: "host_monitor",
            reason: out.stderr,
        });
    }

    let force = signal == "9";
    let sampler = bridge.deps.runtime.local_processes.clone();
    tokio::task::spawn_blocking(move || sampler.lock().kill(pid as u32, force))
        .await
        .map_err(|e| ToolError::ExecutionFailed {
            tool: "host_monitor",
            reason: e.to_string(),
        })?
        .map_err(|e| ToolError::ExecutionFailed {
            tool: "host_monitor",
            reason: e,
        })?;
    Ok(format!("已终止本机进程 {pid}"))
}

async fn probe_host(args: &Value) -> Result<String, ToolError> {
    let host = args["host"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ToolError::InvalidArguments {
            tool: "host_monitor",
            reason: "probe_host 必须提供 host".to_owned(),
        })?
        .to_owned();
    let port = args["port"].as_u64().unwrap_or(22) as u16;
    let addr = format!("{host}:{port}");
    let started = std::time::Instant::now();
    match tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tokio::net::TcpStream::connect(&addr),
    )
    .await
    {
        Ok(Ok(_)) => Ok(format!(
            "TCP 拨测成功: {addr}（耗时 {} ms）",
            started.elapsed().as_millis()
        )),
        Ok(Err(e)) => Ok(format!("TCP 拨测失败: {addr}（{e}）")),
        Err(_) => Ok(format!("TCP 拨测超时（5s）: {addr}")),
    }
}
