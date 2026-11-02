//! 编排（Compose）项目的起停：引擎没有 Compose 的 REST 接口，只能落到 CLI。
//!
//! 设计原理（Why）：Docker/Podman 守护进程都不暴露 `up`/`down`，唯一可靠通道是
//! 官方 CLI。项目的工作目录与配置文件路径直接取自容器标签（引擎给出的绝对路径），
//! 因此这里不接收用户拼接的路径；仍然要求绝对路径且目录存在，避免把 CLI 带到
//! 意料之外的目录去读别人的 compose 文件。
//!
//! 代价与局限（Trade-offs）：依赖宿主机装了 compose 插件（或独立版
//! `docker-compose`），耗时取决于镜像拉取，最长给到 5 分钟；子进程 `kill_on_drop`
//! 加显式 `kill`，取消或超时都不留孤儿；输出只回截断后的尾部，不灌整段构建日志。

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tokio::io::AsyncReadExt;
use transom_core::{ComposeAction, ComposeProject};

use crate::engine::Engine;
use crate::error::ContainerError;

/// 单次编排命令的总超时：拉镜像可能很久。
const COMPOSE_TIMEOUT: Duration = Duration::from_secs(300);
/// 输出回显上限。
const OUTPUT_LIMIT: usize = 8 * 1024;
/// 取消检查节拍：compose 卡住时也要能及时收尾。
const CANCEL_TICK: Duration = Duration::from_millis(250);

/// 执行一次编排项目命令，返回 CLI 输出尾部摘要。
///
/// # Errors
/// 项目缺工作目录或配置文件、目录不存在、CLI 不存在、超时、取消或非零退出码时
/// 返回 [`ContainerError`]，文本带项目名与输出尾部便于定位。
pub async fn run(
    engine: &Engine,
    project: &ComposeProject,
    action: ComposeAction,
    cancel: &Arc<AtomicBool>,
) -> Result<String, ContainerError> {
    validate_project(project, action)?;
    let program = engine.info.engine.kind.cli_program().to_string();
    let working_dir = project
        .working_dir
        .clone()
        .unwrap_or_else(|| "/".to_string());
    log::info!(
        "执行编排命令，项目: {}，命令: {}，工作目录: {}",
        project.name,
        action.label(),
        working_dir
    );
    let first = spawn_compose(&program, project, action, &working_dir, cancel).await?;
    if first.status.success() {
        return Ok(tail_text(&first.stdout, &first.stderr));
    }
    // 只装了独立版 compose 的机器上 `docker compose` 会直接失败，回落到 docker-compose。
    let Some(fallback) = fallback_program(&program) else {
        return Err(ContainerError::Cli {
            command: display_command(&program, action),
            message: tail_text(&first.stdout, &first.stderr),
        });
    };
    let retry = spawn_compose(&fallback, project, action, &working_dir, cancel).await?;
    if retry.status.success() {
        return Ok(tail_text(&retry.stdout, &retry.stderr));
    }
    Err(ContainerError::Cli {
        command: display_command(&fallback, action),
        message: tail_text(&retry.stdout, &retry.stderr),
    })
}

/// 前置校验：拿不到工作目录或配置文件就不值得起子进程。
fn validate_project(project: &ComposeProject, action: ComposeAction) -> Result<(), ContainerError> {
    let operation = format!("编排命令 {}：项目 {}", action.label(), project.name);
    match project.working_dir.as_deref() {
        Some(dir) if dir.starts_with('/') && Path::new(dir).is_dir() => {}
        other => {
            return Err(ContainerError::Unsupported {
                operation: format!("{}：工作目录不可用（{:?}）", operation, other),
            });
        }
    }
    if project.config_files.is_empty() {
        return Err(ContainerError::Unsupported {
            operation: format!("{operation}：容器标签里没有配置文件"),
        });
    }
    Ok(())
}

/// 组装 CLI 参数：工程名与配置文件都作为独立 argv 传入，不经过 Shell。
fn build_args(program: &str, project: &ComposeProject, action: ComposeAction) -> Vec<String> {
    let mut args = Vec::with_capacity(project.config_files.len() * 2 + 4);
    if !is_standalone(program) {
        args.push("compose".to_string());
    }
    args.extend(["-p".to_string(), project.name.clone()]);
    for config in &project.config_files {
        args.extend(["-f".to_string(), config.clone()]);
    }
    args.extend(action.args().iter().map(|arg| (*arg).to_string()));
    args
}

fn is_standalone(program: &str) -> bool {
    program.ends_with("-compose")
}

/// 插件形态才有独立版回落。
fn fallback_program(program: &str) -> Option<String> {
    match program {
        "docker" => Some("docker-compose".to_string()),
        "podman" => Some("podman-compose".to_string()),
        _ => None,
    }
}

/// 日志与错误里显示的命令形态（不含参数细节）。
fn display_command(program: &str, action: ComposeAction) -> String {
    if is_standalone(program) {
        format!("{program} {}", action.subcommand())
    } else {
        format!("{program} compose {}", action.subcommand())
    }
}

struct Completed {
    status: std::process::ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

async fn spawn_compose(
    program: &str,
    project: &ComposeProject,
    action: ComposeAction,
    working_dir: &str,
    cancel: &Arc<AtomicBool>,
) -> Result<Completed, ContainerError> {
    let args = build_args(program, project, action);
    let mut child = tokio::process::Command::new(program)
        .args(&args)
        .current_dir(working_dir)
        .kill_on_drop(true)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|error| ContainerError::Cli {
            command: display_command(program, action),
            message: error.to_string(),
        })?;
    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();
    let stdout_task = tokio::spawn(read_capped(stdout_pipe));
    let stderr_task = tokio::spawn(read_capped(stderr_pipe));

    let deadline = tokio::time::Instant::now() + COMPOSE_TIMEOUT;
    let status = loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill().await;
            return Err(ContainerError::Cancelled);
        }
        if tokio::time::Instant::now() >= deadline {
            let _ = child.kill().await;
            return Err(ContainerError::Cli {
                command: display_command(program, action),
                message: format!("编排命令超过 {} 秒未结束", COMPOSE_TIMEOUT.as_secs()),
            });
        }
        tokio::select! {
            waited = child.wait() => break waited.map_err(|error| ContainerError::Cli {
                command: display_command(program, action),
                message: error.to_string(),
            })?,
            _ = tokio::time::sleep(CANCEL_TICK) => continue,
        }
    };
    // 显式回收：kill_on_drop 只兜住提前返回的路径。
    let _ = child.kill().await;
    Ok(Completed {
        status,
        stdout: stdout_task.await.unwrap_or_default(),
        stderr: stderr_task.await.unwrap_or_default(),
    })
}

/// 读输出并截断到上限：compose 的构建日志可能非常长。
async fn read_capped(pipe: Option<impl tokio::io::AsyncRead + Unpin + Send>) -> Vec<u8> {
    let Some(mut pipe) = pipe else {
        return Vec::new();
    };
    let mut buffer = Vec::with_capacity(1024);
    let mut chunk = [0u8; 4096];
    while let Ok(count) = pipe.read(&mut chunk).await {
        if count == 0 {
            break;
        }
        if buffer.len() < OUTPUT_LIMIT {
            buffer.extend_from_slice(&chunk[..count]);
        }
    }
    buffer
}

/// 输出尾部摘要：compose 把进度与错误都写在 stderr，优先取它。
fn tail_text(stdout: &[u8], stderr: &[u8]) -> String {
    let preferred = if stderr.iter().any(|byte| !byte.is_ascii_whitespace()) {
        stderr
    } else {
        stdout
    };
    let text = String::from_utf8_lossy(preferred).trim().to_string();
    if text.is_empty() {
        return "无输出".to_string();
    }
    if text.len() > 1024 {
        // 按字节切片前先把起点推到字符边界，避免切断多字节字符。
        let mut start = text.len() - 1024;
        while !text.is_char_boundary(start) {
            start += 1;
        }
        return format!("…{}", &text[start..]);
    }
    text
}
