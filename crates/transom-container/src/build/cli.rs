//! 基于 docker/podman build 子进程的兜底镜像构建。

use crate::cli::CliBackend;
use crate::error::ContainerError;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use transom_core::ContainerBuildSpec;

/// CLI 构建总超时：构建可能持续很久，但必须有上限，避免子进程永久挂住。
const CLI_BUILD_TIMEOUT: Duration = Duration::from_secs(60 * 60);
/// 基于宿主机 CLI 执行镜像构建。
pub async fn build_image_cli(
    cli: &CliBackend,
    spec: &ContainerBuildSpec,
    cancel: &Arc<AtomicBool>,
    progress: &mut (dyn FnMut(String) + Send),
) -> Result<(), ContainerError> {
    let mut args = vec!["build".to_string()];
    if let Some(df) = &spec.dockerfile
        && !df.is_empty()
    {
        args.push("-f".to_string());
        args.push(df.clone());
    }
    for tag in &spec.tags {
        if !tag.is_empty() {
            args.push("-t".to_string());
            args.push(tag.clone());
        }
    }
    if spec.no_cache {
        args.push("--no-cache".to_string());
    }
    if let Some(target) = &spec.target
        && !target.is_empty()
    {
        args.push("--target".to_string());
        args.push(target.clone());
    }
    if let Some(platform) = &spec.platform
        && !platform.is_empty()
    {
        args.push("--platform".to_string());
        args.push(platform.clone());
    }
    for (key, val) in &spec.build_args {
        args.push("--build-arg".to_string());
        args.push(format!("{key}={val}"));
    }
    args.push(spec.context_path.clone());

    let mut cmd = cli.command(&args);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

    let command_line = format!("docker {}", args.join(" "));
    let mut child = crate::cli::spawn_grouped(&mut cmd).map_err(|error| ContainerError::Cli {
        command: command_line.clone(),
        message: format!("启动构建子进程失败: {error}"),
    })?;

    let stdout = child.inner().stdout.take();
    let stderr = child.inner().stderr.take();

    let mut out_reader = stdout.map(|s| BufReader::new(s).lines());
    let mut err_reader = stderr.map(|s| BufReader::new(s).lines());

    let mut stdout_done = out_reader.is_none();
    let mut stderr_done = err_reader.is_none();
    let deadline = tokio::time::Instant::now() + CLI_BUILD_TIMEOUT;

    while !stdout_done || !stderr_done {
        if cancel.load(Ordering::Relaxed) {
            crate::cli::terminate_child(&mut child).await;
            return Err(ContainerError::Cancelled);
        }
        if tokio::time::Instant::now() >= deadline {
            crate::cli::terminate_child(&mut child).await;
            return Err(ContainerError::Cli {
                command: command_line,
                message: format!("构建超过 {} 秒未结束", CLI_BUILD_TIMEOUT.as_secs()),
            });
        }
        tokio::select! {
            res = async {
                match &mut out_reader {
                    Some(r) => r.next_line().await,
                    None => Ok(None),
                }
            }, if !stdout_done => {
                match res {
                    Ok(Some(line)) => progress(line),
                    Ok(None) => stdout_done = true,
                    Err(e) => {
                        log::warn!("读取构建 stdout 异常: {e}");
                        stdout_done = true;
                    }
                }
            }
            res = async {
                match &mut err_reader {
                    Some(r) => r.next_line().await,
                    None => Ok(None),
                }
            }, if !stderr_done => {
                match res {
                    Ok(Some(line)) => progress(line),
                    Ok(None) => stderr_done = true,
                    Err(e) => {
                        log::warn!("读取构建 stderr 异常: {e}");
                        stderr_done = true;
                    }
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(200)) => {}
        }
    }

    let status = child.wait().await.map_err(|error| ContainerError::Cli {
        command: format!("docker {}", args.join(" ")),
        message: format!("等待构建子进程退出失败: {error}"),
    })?;

    if !status.success() {
        return Err(ContainerError::Cli {
            command: format!("docker {}", args.join(" ")),
            message: format!("构建失败，进程退出状态码: {:?}", status.code()),
        });
    }

    Ok(())
}
