//! 本机 Shell 命令与本机文件系统操作（供统一目标路由 `target = "local"` 调用）。

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use serde_json::Value;
use tokio::io::AsyncReadExt;
use transom_ai::ToolError;
use transom_core::AiManagedTaskStatus;

use super::ControllerAppActionBridge;

pub(super) const MAX_LOCAL_READ_BYTES: usize = 256 * 1024;

pub(super) async fn exec_local_shell(
    bridge: &ControllerAppActionBridge,
    command: &str,
    args: &Value,
    timeout: Duration,
) -> Result<String, ToolError> {
    let mut cmd = build_local_shell_command(command);
    if let Some(cwd) = args["cwd"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let dir = Path::new(cwd);
        if !dir.is_dir() {
            return Err(ToolError::InvalidArguments {
                tool: "remote_exec",
                reason: format!("指定工作目录不存在或不是目录: {cwd}"),
            });
        }
        cmd.current_dir(dir);
    }
    cmd.stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let title = command.chars().take(60).collect::<String>();
    let task = bridge.deps.tasks.start_task(
        "local_exec",
        &title,
        "本机 (Local)",
        &bridge.deps.cancel,
        None,
    );

    let mut child = cmd.spawn().map_err(|e| ToolError::ExecutionFailed {
        tool: "remote_exec",
        reason: format!("启动本机命令失败: {e}"),
    })?;

    let mut stdout_pipe = child.stdout.take();
    let mut stderr_pipe = child.stderr.take();
    let mut stdout_buf = String::new();
    let mut stderr_buf = String::new();
    let mut out_chunk = [0u8; 4096];
    let mut err_chunk = [0u8; 4096];

    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if stdout_pipe.is_none() && stderr_pipe.is_none() {
            break;
        }
        tokio::select! {
            _ = task.detach.cancelled() => {
                let bg_id = task.task_id.clone();
                tokio::spawn(async move {
                    let mut out_chunk = [0u8; 4096];
                    let mut err_chunk = [0u8; 4096];
                    while stdout_pipe.is_some() || stderr_pipe.is_some() {
                        tokio::select! {
                            _ = task.cancel.cancelled() => {
                                let _ = child.kill().await;
                                let _ = child.wait().await;
                                task.finish(AiManagedTaskStatus::Cancelled, None, "本机命令已取消");
                                return;
                            }
                            res = async {
                                match stdout_pipe.as_mut() {
                                    Some(p) => p.read(&mut out_chunk).await,
                                    None => std::future::pending().await,
                                }
                            }, if stdout_pipe.is_some() => match res {
                                Ok(0) | Err(_) => stdout_pipe = None,
                                Ok(n) => task.emit_chunk(&String::from_utf8_lossy(&out_chunk[..n])),
                            },
                            res = async {
                                match stderr_pipe.as_mut() {
                                    Some(p) => p.read(&mut err_chunk).await,
                                    None => std::future::pending().await,
                                }
                            }, if stderr_pipe.is_some() => match res {
                                Ok(0) | Err(_) => stderr_pipe = None,
                                Ok(n) => task.emit_chunk(&String::from_utf8_lossy(&err_chunk[..n])),
                            },
                        }
                    }
                    let code = child.wait().await.ok().and_then(|s| s.code()).unwrap_or(-1);
                    let st = if code == 0 { AiManagedTaskStatus::Succeeded } else { AiManagedTaskStatus::Failed };
                    task.finish(st, Some(code), format!("退出码: {code}"));
                });
                return Ok(format!(
                    "本机命令已转入后台托管运行（task_id: {bg_id}），可在任务中心实时查看输出，主对话可继续进行。"
                ));
            }
            _ = task.cancel.cancelled() => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                task.finish(AiManagedTaskStatus::Cancelled, None, "本机命令已取消");
                return Err(ToolError::Cancelled);
            }
            _ = tokio::time::sleep_until(deadline) => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                task.finish(AiManagedTaskStatus::Failed, None, "本机命令执行超时");
                return Err(ToolError::ExecutionTimeout {
                    timeout_secs: timeout.as_secs(),
                });
            }
            res = async {
                match stdout_pipe.as_mut() {
                    Some(p) => p.read(&mut out_chunk).await,
                    None => std::future::pending().await,
                }
            }, if stdout_pipe.is_some() => {
                match res {
                    Ok(0) | Err(_) => stdout_pipe = None,
                    Ok(n) => {
                        let s = String::from_utf8_lossy(&out_chunk[..n]);
                        stdout_buf.push_str(&s);
                        task.emit_chunk(&s);
                    }
                }
            }
            res = async {
                match stderr_pipe.as_mut() {
                    Some(p) => p.read(&mut err_chunk).await,
                    None => std::future::pending().await,
                }
            }, if stderr_pipe.is_some() => {
                match res {
                    Ok(0) | Err(_) => stderr_pipe = None,
                    Ok(n) => {
                        let s = String::from_utf8_lossy(&err_chunk[..n]);
                        stderr_buf.push_str(&s);
                        task.emit_chunk(&s);
                    }
                }
            }
        }
    }

    let status = child.wait().await.map_err(|e| ToolError::ExecutionFailed {
        tool: "remote_exec",
        reason: format!("等待本机命令结果失败: {e}"),
    })?;
    let code = status.code().unwrap_or(-1);
    let task_status = if code == 0 {
        AiManagedTaskStatus::Succeeded
    } else {
        AiManagedTaskStatus::Failed
    };
    task.finish(task_status, Some(code), format!("退出码: {code}"));
    Ok(format_exit_output(code, &stdout_buf, &stderr_buf))
}

pub(super) async fn handle_local_fs(
    action: &str,
    path: &str,
    args: &Value,
) -> Result<String, ToolError> {
    let p = Path::new(path);
    match action {
        "list" => {
            let mut rd = tokio::fs::read_dir(p).await.map_err(map_fs_err)?;
            let mut lines = Vec::new();
            while let Some(entry) = rd.next_entry().await.map_err(map_fs_err)? {
                let meta = entry.metadata().await.map_err(map_fs_err)?;
                let kind = if meta.is_dir() { "DIR " } else { "FILE" };
                lines.push(format!(
                    "{kind} {:>10} {}",
                    meta.len(),
                    entry.file_name().to_string_lossy()
                ));
                if lines.len() >= 200 {
                    break;
                }
            }
            Ok(lines.join("\n"))
        }
        "read" => {
            let bytes = tokio::fs::read(p).await.map_err(map_fs_err)?;
            let slice = &bytes[..bytes.len().min(MAX_LOCAL_READ_BYTES)];
            Ok(String::from_utf8_lossy(slice).into_owned())
        }
        "search" => {
            let pat = args["pattern"].as_str().unwrap_or("");
            let content = tokio::fs::read_to_string(p).await.map_err(map_fs_err)?;
            let hits: Vec<String> = content
                .lines()
                .enumerate()
                .filter(|(_, line)| line.contains(pat))
                .take(120)
                .map(|(i, line)| format!("{}:{line}", i + 1))
                .collect();
            Ok(hits.join("\n"))
        }
        "mkdir" => {
            tokio::fs::create_dir_all(p).await.map_err(map_fs_err)?;
            Ok(format!("已创建本机目录: {path}"))
        }
        "edit" => {
            let content = args["content"].as_str().unwrap_or("");
            if let Some(parent) = p.parent() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .map_err(map_fs_err)?;
            }
            tokio::fs::write(p, content).await.map_err(map_fs_err)?;
            Ok(format!("已写入本机文件: {path}"))
        }
        "rename" => {
            let to = req_str(args, "target_path", "remote_fs")?;
            tokio::fs::rename(p, to).await.map_err(map_fs_err)?;
            Ok(format!("已重命名本机路径: {path} -> {to}"))
        }
        "delete" => {
            let rec = args["recursive"].as_bool().unwrap_or(false);
            let meta = tokio::fs::metadata(p).await.map_err(map_fs_err)?;
            if meta.is_dir() {
                if rec {
                    tokio::fs::remove_dir_all(p).await.map_err(map_fs_err)?;
                } else {
                    tokio::fs::remove_dir(p).await.map_err(map_fs_err)?;
                }
            } else {
                tokio::fs::remove_file(p).await.map_err(map_fs_err)?;
            }
            Ok(format!("已删除本机路径: {path}"))
        }
        "chmod" => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                let raw = req_str(args, "mode", "remote_fs")?;
                let mode = u32::from_str_radix(raw.trim_start_matches('0'), 8).map_err(|_| {
                    ToolError::InvalidArguments {
                        tool: "remote_fs",
                        reason: format!("无效权限: {raw}"),
                    }
                })?;
                tokio::fs::set_permissions(p, std::fs::Permissions::from_mode(mode))
                    .await
                    .map_err(map_fs_err)?;
                Ok(format!("已修改本机权限 {raw}: {path}"))
            }
            #[cfg(not(unix))]
            Err(ToolError::ExecutionFailed {
                tool: "remote_fs",
                reason: "当前平台不支持 chmod".to_owned(),
            })
        }
        other => Err(ToolError::InvalidArguments {
            tool: "remote_fs",
            reason: format!("未知 remote_fs 动作: {other}"),
        }),
    }
}

#[cfg(not(windows))]
fn build_local_shell_command(command: &str) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new("sh");
    cmd.arg("-lc").arg(command);
    cmd
}

#[cfg(windows)]
fn build_local_shell_command(command: &str) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new("cmd");
    cmd.arg("/C").arg(command);
    cmd
}

pub(super) fn format_exit_output(code: i32, stdout: &str, stderr: &str) -> String {
    let mut text = format!("退出码: {code}\n{stdout}");
    if !stderr.is_empty() {
        if !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str("[stderr]\n");
        text.push_str(stderr);
    }
    text
}

fn map_fs_err(err: std::io::Error) -> ToolError {
    ToolError::ExecutionFailed {
        tool: "remote_fs",
        reason: err.to_string(),
    }
}

pub(super) fn req_str<'a>(
    args: &'a Value,
    key: &str,
    tool: &'static str,
) -> Result<&'a str, ToolError> {
    args[key]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ToolError::InvalidArguments {
            tool,
            reason: format!("缺少必要参数 {key}"),
        })
}
