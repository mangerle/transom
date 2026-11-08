//! 容器 CLI 子进程树生命周期管理。
//!
//! 设计原理（Why）：容器命令经常继续派生 buildx、WSL 转发进程等孙进程，只终止
//! 直接子进程会留下孤儿，并让 stdout/stderr 管道无法关闭。统一通过
//! `command-group` 在 Unix 建立独立进程组、在 Windows 建立 Job Object，使超时、
//! 取消与会话关闭都能一次回收整棵进程树。

use command_group::{AsyncCommandGroup, AsyncGroupChild, CommandGroup, GroupChild};
use std::io::{self, Read};
use std::process::{Output, Stdio};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use tokio::io::AsyncReadExt;
use tokio::task::JoinHandle as AsyncJoinHandle;

/// 子进程树状态轮询与管道排空的检查节拍。
const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(10);

/// 以平台进程组 / Windows Job Object 启动异步 CLI 子进程。
pub(crate) fn spawn_grouped(command: &mut tokio::process::Command) -> io::Result<AsyncGroupChild> {
    command.group().kill_on_drop(true).spawn()
}

/// 终止整个异步 CLI 子进程组并等待回收。
pub(crate) async fn terminate_child(child: &mut AsyncGroupChild) {
    let _ = child.kill().await;
    let _ = child.wait().await;
}

/// 在超时上限内等待异步 CLI 子进程树结束，并排空标准输出与标准错误。
///
/// # Errors
/// 等待进程失败、读取输出失败、输出任务异常或命令超时时返回错误。
pub(crate) async fn wait_for_group_output(
    child: &mut AsyncGroupChild,
    timeout: Duration,
) -> io::Result<Output> {
    let stdout_reader = spawn_async_pipe_reader(child.inner().stdout.take());
    let stderr_reader = spawn_async_pipe_reader(child.inner().stderr.take());
    let deadline = Instant::now() + timeout;
    let mut status = None;

    loop {
        if status.is_none() {
            match child.try_wait() {
                Ok(current) => status = current,
                Err(error) => {
                    terminate_child(child).await;
                    abort_async_pipe_reader(stdout_reader).await;
                    abort_async_pipe_reader(stderr_reader).await;
                    return Err(error);
                }
            }
        }
        let stdout_finished = stdout_reader
            .as_ref()
            .is_none_or(AsyncJoinHandle::is_finished);
        let stderr_finished = stderr_reader
            .as_ref()
            .is_none_or(AsyncJoinHandle::is_finished);
        if status.is_some() && stdout_finished && stderr_finished {
            break;
        }
        if Instant::now() >= deadline {
            terminate_child(child).await;
            abort_async_pipe_reader(stdout_reader).await;
            abort_async_pipe_reader(stderr_reader).await;
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("命令超过 {} 秒未完成", timeout.as_secs()),
            ));
        }
        tokio::time::sleep(PROCESS_POLL_INTERVAL).await;
    }

    let stdout = join_async_pipe_reader(stdout_reader)
        .await?
        .unwrap_or_default();
    let stderr = join_async_pipe_reader(stderr_reader)
        .await?
        .unwrap_or_default();
    let status = status.ok_or_else(|| io::Error::other("命令退出状态不可用"))?;
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

/// 在超时上限内执行同步 CLI 命令，并回收整个进程组。
///
/// 设计原理（Why）：仅等待组长退出并不代表管道已关闭，组长退出后其孙进程仍可能
/// 持有 stdout/stderr。本函数并发排空两个管道，并在总时限到达时终止整个进程组，
/// 从而避免 `wait_with_output` 在异常子进程树上无限阻塞。
///
/// # Errors
/// 子进程无法启动、管道线程无法创建、读取输出失败或命令超时时返回错误。
pub(crate) fn run_grouped_output(
    command: &mut std::process::Command,
    timeout: Duration,
) -> io::Result<Output> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command.group_spawn()?;
    let stdout_reader = match spawn_pipe_reader(child.inner().stdout.take()) {
        Ok(reader) => reader,
        Err(error) => {
            terminate_grouped_child(&mut child);
            return Err(error);
        }
    };
    let stderr_reader = match spawn_pipe_reader(child.inner().stderr.take()) {
        Ok(reader) => reader,
        Err(error) => {
            terminate_grouped_child(&mut child);
            let _ = join_pipe_reader(stdout_reader);
            return Err(error);
        }
    };

    let deadline = Instant::now() + timeout;
    let mut status = None;
    loop {
        if status.is_none() {
            status = match child.try_wait() {
                Ok(status) => status,
                Err(error) => {
                    terminate_grouped_child(&mut child);
                    let _ = join_pipe_reader(stdout_reader);
                    let _ = join_pipe_reader(stderr_reader);
                    return Err(error);
                }
            };
        }
        let stdout_finished = stdout_reader.as_ref().is_none_or(JoinHandle::is_finished);
        let stderr_finished = stderr_reader.as_ref().is_none_or(JoinHandle::is_finished);
        if status.is_some() && stdout_finished && stderr_finished {
            break;
        }
        if Instant::now() >= deadline {
            terminate_grouped_child(&mut child);
            let _ = join_pipe_reader(stdout_reader);
            let _ = join_pipe_reader(stderr_reader);
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("命令超过 {} 秒未完成", timeout.as_secs()),
            ));
        }
        std::thread::sleep(PROCESS_POLL_INTERVAL);
    }

    let stdout = join_pipe_reader(stdout_reader)?.unwrap_or_default();
    let stderr = join_pipe_reader(stderr_reader)?.unwrap_or_default();
    let status = status.ok_or_else(|| io::Error::other("命令退出状态不可用"))?;
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

/// 终止同步 CLI 子进程组并等待回收。
fn terminate_grouped_child(child: &mut GroupChild) {
    let _ = child.kill();
    let _ = child.wait();
}

/// 启动独立线程排空管道，避免父进程与大量输出互相阻塞。
fn spawn_pipe_reader<R>(reader: Option<R>) -> io::Result<Option<JoinHandle<io::Result<Vec<u8>>>>>
where
    R: Read + Send + 'static,
{
    reader
        .map(|mut reader| {
            std::thread::Builder::new()
                .name("transom-container-cli-output".to_string())
                .spawn(move || {
                    let mut bytes = Vec::with_capacity(8192);
                    reader.read_to_end(&mut bytes)?;
                    Ok(bytes)
                })
        })
        .transpose()
}

/// 等待管道线程结束并归一化线程 panic。
fn join_pipe_reader(
    reader: Option<JoinHandle<io::Result<Vec<u8>>>>,
) -> io::Result<Option<Vec<u8>>> {
    let Some(handle) = reader else {
        return Ok(None);
    };
    handle
        .join()
        .map_err(|_| io::Error::other("容器 CLI 输出线程异常退出"))?
        .map(Some)
}

/// 启动 Tokio 任务排空异步管道。
fn spawn_async_pipe_reader<R>(reader: Option<R>) -> Option<AsyncJoinHandle<io::Result<Vec<u8>>>>
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    reader.map(|mut reader| {
        tokio::spawn(async move {
            let mut bytes = Vec::with_capacity(8192);
            reader.read_to_end(&mut bytes).await?;
            Ok(bytes)
        })
    })
}

/// 等待异步管道任务结束并归一化任务取消与 panic。
async fn join_async_pipe_reader(
    reader: Option<AsyncJoinHandle<io::Result<Vec<u8>>>>,
) -> io::Result<Option<Vec<u8>>> {
    let Some(handle) = reader else {
        return Ok(None);
    };
    handle
        .await
        .map_err(|error| io::Error::other(format!("容器 CLI 输出任务异常退出: {error}")))?
        .map(Some)
}

/// 主动取消异步管道任务，避免终止子进程后继续占用运行时资源。
async fn abort_async_pipe_reader(reader: Option<AsyncJoinHandle<io::Result<Vec<u8>>>>) {
    if let Some(handle) = reader {
        handle.abort();
        let _ = handle.await;
    }
}

#[cfg(test)]
mod tests {
    use super::run_grouped_output;
    use std::process::Command;
    use std::time::Duration;
    #[cfg(unix)]
    use std::time::Instant;

    #[test]
    fn grouped_output_captures_both_streams_and_status() {
        let mut command = exit_with_output_command();
        let output = run_grouped_output(&mut command, Duration::from_secs(3))
            .expect("进程组输出命令必须成功执行");
        assert_eq!(output.status.code(), Some(7));
        assert!(String::from_utf8_lossy(&output.stdout).contains("stdout"));
        assert!(String::from_utf8_lossy(&output.stderr).contains("stderr"));
    }

    #[cfg(unix)]
    #[test]
    fn timeout_kills_descendant_that_keeps_output_pipes_open() {
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 5 & exit 0"]);
        let start = Instant::now();
        let error = run_grouped_output(&mut command, Duration::from_millis(200))
            .expect_err("仍持有管道的孙进程必须触发超时");
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert!(
            start.elapsed() < Duration::from_secs(3),
            "终止进程组后不应继续等待孙进程"
        );
    }

    #[cfg(unix)]
    fn exit_with_output_command() -> Command {
        let mut command = Command::new("sh");
        command.args(["-c", "printf stdout; printf stderr >&2; exit 7"]);
        command
    }

    #[cfg(windows)]
    fn exit_with_output_command() -> Command {
        let mut command = Command::new("cmd");
        command.args(["/C", "echo stdout& echo stderr 1>&2& exit /b 7"]);
        command
    }
}
