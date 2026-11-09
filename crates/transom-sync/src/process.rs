//! 跨平台子进程执行：并发排空输出管道，并在超时后终止、回收进程。
//!
//! 设计原理（Why）：Git 同步运行在阻塞线程中，不能依赖 Unix 才有的 GNU
//! `timeout` 命令；直接调用 `Child::wait_with_output` 又无法同时施加超时。
//! 本模块把 stdout/stderr 分别交给读取线程，主线程轮询子进程状态，超时后
//! 先终止再等待，最后汇合读取线程，保证管道排空、子进程收割且无孤儿线程。
//!
//! 代价与局限：每次调用需要两个短生命周期线程；Git 同步是低频阻塞操作，
//! 这点创建成本远低于跨平台外部命令依赖带来的不可用风险。

use command_group::{CommandGroup, GroupChild};
use std::io::{self, Read};
use std::process::{Command, ExitStatus, Output};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// 子进程轮询节拍：足够及时地响应超时，又不会在长任务上空转消耗 CPU。
const POLL_INTERVAL: Duration = Duration::from_millis(20);

/// 子进程执行失败的类型。
#[derive(Debug)]
pub(crate) enum RunError {
    /// 启动、等待或读取管道失败。
    Io(io::Error),
    /// 子进程超过截止时间，已被终止并回收。
    Timeout,
}

impl From<io::Error> for RunError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// 执行命令并捕获有上限前的标准输出与标准错误。
///
/// stdout/stderr 由独立线程并发排空，避免任一管道写满后子进程阻塞。
/// 超时路径会先 `kill`、再 `wait`，最后等待读取线程退出，确保返回时没有
/// 遗留子进程或后台线程。
pub(crate) fn output_with_timeout(
    mut command: Command,
    timeout: Duration,
) -> Result<Output, RunError> {
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = command.group_spawn()?;
    let stdout = spawn_reader(child.inner().stdout.take())?;
    let stderr = match spawn_reader(child.inner().stderr.take()) {
        Ok(reader) => reader,
        Err(error) => {
            cleanup_child(&mut child);
            let _ = join_reader(stdout);
            return Err(error.into());
        }
    };

    let status = wait_for_exit(&mut child, timeout);
    let status = match status {
        Ok(status) => status,
        Err(error) => {
            cleanup_child(&mut child);
            let _ = join_reader(stdout);
            let _ = join_reader(stderr);
            return Err(error);
        }
    };
    let stdout = join_reader(stdout)?;
    let stderr = join_reader(stderr)?;
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

fn spawn_reader<R>(reader: Option<R>) -> io::Result<JoinHandle<io::Result<Vec<u8>>>>
where
    R: Read + Send + 'static,
{
    let mut reader = reader;
    thread::Builder::new()
        .name("transom-sync-pipe".to_string())
        .spawn(move || {
            let mut bytes = Vec::with_capacity(8 * 1024);
            if let Some(reader) = reader.as_mut() {
                reader.read_to_end(&mut bytes)?;
            }
            Ok(bytes)
        })
}

fn wait_for_exit(child: &mut GroupChild, timeout: Duration) -> Result<ExitStatus, RunError> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        let now = Instant::now();
        if now >= deadline {
            return Err(RunError::Timeout);
        }
        thread::sleep(POLL_INTERVAL.min(deadline.saturating_duration_since(now)));
    }
}

fn cleanup_child(child: &mut GroupChild) {
    let _ = child.kill();
    let _ = child.wait();
}

fn join_reader(reader: JoinHandle<io::Result<Vec<u8>>>) -> io::Result<Vec<u8>> {
    reader
        .join()
        .map_err(|_| io::Error::other("子进程输出读取线程异常终止"))?
}

#[cfg(test)]
mod tests {
    use super::{RunError, output_with_timeout};
    use std::process::Command;
    use std::time::Duration;

    #[cfg(unix)]
    fn command(script: &str) -> Command {
        let mut command = Command::new("sh");
        command.args(["-c", script]);
        command
    }

    #[cfg(windows)]
    fn command(script: &str) -> Command {
        let mut command = Command::new("cmd");
        command.args(["/C", script]);
        command
    }

    #[test]
    fn captures_both_streams_without_pipe_deadlock() {
        #[cfg(unix)]
        let script = "printf stdout; printf stderr >&2";
        #[cfg(windows)]
        // set /p 在 NUL 输入读到 EOF 时可能返回非零；本测试只验证双管道排空，
        // 因此两个输出都完成后显式以成功状态退出。
        let script = "(<nul set /p =stdout) & (<nul set /p =stderr 1>&2) & exit /b 0";
        let output = output_with_timeout(command(script), Duration::from_secs(5)).unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"stdout");
        assert_eq!(output.stderr, b"stderr");
    }

    #[test]
    fn timeout_kills_and_reaps_process() {
        #[cfg(unix)]
        let script = "sleep 5";
        #[cfg(windows)]
        let script = "ping -n 6 127.0.0.1 >nul";
        let error = output_with_timeout(command(script), Duration::from_millis(100)).unwrap_err();
        assert!(matches!(error, RunError::Timeout));
    }
}
