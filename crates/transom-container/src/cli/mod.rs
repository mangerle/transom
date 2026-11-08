//! CLI 兜底后端。
//!
//! 设计原理（Why）：Windows 的 WSL 发行版内 Docker、WSLC 以及无权访问
//! socket 的 Linux 用户，都无法直连 HTTP 端点；`docker`/`podman`/`wslc`
//! 的 CLI 是唯一稳定入口。CLI 后端只实现能力子集（无 resize、无归档 API），
//! 由能力矩阵显式暴露，而不是让用户在点击后才收到失败。
//!
//! 安全约束：所有参数以 argv 数组传递，绝不拼接 Shell 字符串；输出解析
//! 只依赖机器可读的 `--format '{{json .}}'`，不解析人类可读表格。
//!
//! 子命令按资源域拆到子模块，共享的子进程执行与取消管线留在本模块。

use crate::discovery::CliSpec;
use crate::error::ContainerError;
use serde::de::DeserializeOwned;
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;
use transom_core::ContainerEngineKind;

/// 短查询命令超时；列表、检视、启停等交互式操作使用这一档。
const CLI_TIMEOUT: Duration = Duration::from_secs(20);
/// 镜像拉取/推送超时：大镜像可能持续很久，不能用 20 秒把正常传输杀掉。
const CLI_IMAGE_TRANSFER_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// 镜像导出/导入超时：归档读写取决于磁盘与镜像体积。
pub(crate) const CLI_ARCHIVE_TIMEOUT: Duration = Duration::from_secs(60 * 60);
/// 系统级清理超时：需要删除多类资源，允许更长的执行窗口。
const CLI_PRUNE_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// 系统磁盘占用查询超时：镜像/卷很多时引擎统计可能较慢。
const CLI_SYSTEM_DF_TIMEOUT: Duration = Duration::from_secs(60);
/// 单个镜像删除超时：大镜像的层回收可能慢于普通查询。
const CLI_IMAGE_REMOVE_TIMEOUT: Duration = Duration::from_secs(120);
/// 单条命令输出上限：防止异常命令把整段输出读进内存。
const CLI_OUTPUT_LIMIT: usize = 4 * 1024 * 1024;
/// 取消与超时的检查节拍。
const CLI_TICK: Duration = Duration::from_millis(100);

mod containers;
mod images;
mod networks;
mod pods;
mod process;
mod system;
mod volumes;

pub(crate) use process::{
    run_grouped_output, spawn_grouped, terminate_child, wait_for_group_output,
};

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

/// 带截断标记的原始输出。
struct CappedOutput {
    bytes: Vec<u8>,
    truncated: bool,
}

/// 排空管道，只保留前 `CLI_OUTPUT_LIMIT` 字节，剩余数据继续读取并丢弃。
///
/// 设计原理（Why）：不能只读到上限就停止轮询，否则子进程会因管道写满而阻塞；
/// 必须持续消费到 EOF，只对保留区做截断。
async fn read_capped<R>(reader: Option<R>) -> io::Result<CappedOutput>
where
    R: AsyncRead + Unpin,
{
    let Some(mut reader) = reader else {
        return Ok(CappedOutput {
            bytes: Vec::new(),
            truncated: false,
        });
    };
    let mut bytes = Vec::with_capacity(8192);
    let mut buffer = [0u8; 8192];
    let mut truncated = false;
    loop {
        let count = reader.read(&mut buffer).await?;
        if count == 0 {
            break;
        }
        if bytes.len() >= CLI_OUTPUT_LIMIT {
            truncated = true;
            continue;
        }
        let remaining = CLI_OUTPUT_LIMIT - bytes.len();
        let take = remaining.min(count);
        bytes.extend_from_slice(&buffer[..take]);
        if take < count {
            truncated = true;
        }
    }
    Ok(CappedOutput { bytes, truncated })
}

/// 等待输出读取任务并把任务 panic/管道错误收敛为带命令上下文的 CLI 错误。
async fn join_output(
    handle: tokio::task::JoinHandle<io::Result<CappedOutput>>,
    command: &str,
    stream: &str,
) -> Result<CappedOutput, ContainerError> {
    let result = handle.await.map_err(|error| ContainerError::Cli {
        command: command.to_string(),
        message: format!("读取命令{stream}任务失败: {error}"),
    })?;
    result.map_err(|error| ContainerError::Cli {
        command: command.to_string(),
        message: format!("读取命令{stream}失败: {error}"),
    })
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

impl CliBackend {
    pub fn new(kind: ContainerEngineKind, spec: CliSpec) -> Self {
        Self { kind, spec }
    }

    pub fn command(&self, args: &[String]) -> Command {
        let mut command = Command::new(&self.spec.program);
        command.args(&self.spec.prefix).args(args);
        command
    }

    fn describe(&self, args: &[String]) -> String {
        self.spec.describe(args)
    }

    pub async fn run(&self, args: &[String]) -> Result<CliOutput, ContainerError> {
        self.run_controlled(args, CLI_TIMEOUT, None).await
    }

    pub(crate) async fn run_with_timeout(
        &self,
        args: &[String],
        timeout: Duration,
    ) -> Result<CliOutput, ContainerError> {
        self.run_controlled(args, timeout, None).await
    }

    pub(crate) async fn run_checked(&self, args: &[String]) -> Result<String, ContainerError> {
        self.run_checked_controlled(args, CLI_TIMEOUT, None).await
    }

    pub(crate) async fn run_checked_with_timeout(
        &self,
        args: &[String],
        timeout: Duration,
    ) -> Result<String, ContainerError> {
        self.run_checked_controlled(args, timeout, None).await
    }

    pub(crate) async fn run_checked_cancellable(
        &self,
        args: &[String],
        timeout: Duration,
        cancel: &Arc<AtomicBool>,
    ) -> Result<String, ContainerError> {
        self.run_checked_controlled(args, timeout, Some(cancel))
            .await
    }

    async fn run_checked_controlled(
        &self,
        args: &[String],
        timeout: Duration,
        cancel: Option<&Arc<AtomicBool>>,
    ) -> Result<String, ContainerError> {
        let output = self.run_controlled(args, timeout, cancel).await?;
        if !output.success {
            return Err(ContainerError::Cli {
                command: self.describe(args),
                message: output.stderr.trim().to_string(),
            });
        }
        Ok(output.stdout)
    }

    async fn run_controlled(
        &self,
        args: &[String],
        timeout: Duration,
        cancel: Option<&Arc<AtomicBool>>,
    ) -> Result<CliOutput, ContainerError> {
        let description = self.describe(args);
        let mut command = self.command(args);
        command
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let mut child = spawn_grouped(&mut command).map_err(|error| ContainerError::Cli {
            command: description.clone(),
            message: error.to_string(),
        })?;
        let stdout_task = tokio::spawn(read_capped(child.inner().stdout.take()));
        let stderr_task = tokio::spawn(read_capped(child.inner().stderr.take()));
        let deadline = tokio::time::Instant::now() + timeout;

        let outcome = loop {
            if cancel.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
                terminate_child(&mut child).await;
                break Err(ContainerError::Cancelled);
            }
            if tokio::time::Instant::now() >= deadline {
                terminate_child(&mut child).await;
                break Err(ContainerError::Cli {
                    command: description.clone(),
                    message: format!("命令执行超时（{} 秒）", timeout.as_secs()),
                });
            }
            tokio::select! {
                status = child.wait() => {
                    match status {
                        Ok(status) => break Ok(status),
                        Err(error) => {
                            // 等待失败时子进程状态未知，先终止再回收，避免输出任务永久等待管道 EOF。
                            terminate_child(&mut child).await;
                            break Err(ContainerError::Cli {
                                command: description.clone(),
                                message: format!("等待命令退出失败: {error}"),
                            });
                        }
                    }
                }
                _ = tokio::time::sleep(CLI_TICK) => continue,
            }
        };

        let stdout = join_output(stdout_task, &description, "标准输出").await;
        let stderr = join_output(stderr_task, &description, "标准错误").await;
        let stdout = stdout?;
        let stderr = stderr?;
        if stdout.truncated || stderr.truncated {
            log::warn!(
                "容器 CLI 输出超过上限 {} 字节，已截断，命令: {description}",
                CLI_OUTPUT_LIMIT
            );
        }
        let status = outcome?;
        Ok(CliOutput {
            success: status.success(),
            stdout: String::from_utf8_lossy(&stdout.bytes).to_string(),
            stderr: String::from_utf8_lossy(&stderr.bytes).to_string(),
        })
    }

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
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_human_sizes() {
        assert_eq!(parse_human_size("512B"), Some(512));
        assert_eq!(parse_human_size("1.5kB"), Some(1500));
        assert_eq!(parse_human_size("2MB"), Some(2_000_000));
        assert_eq!(parse_human_size("1GiB"), Some(1_073_741_824));
        assert_eq!(parse_human_size("unknown"), None);
    }

    #[cfg(unix)]
    fn sh_backend(program: &str) -> super::CliBackend {
        super::CliBackend::new(
            transom_core::ContainerEngineKind::Docker,
            crate::discovery::CliSpec {
                program: program.to_string(),
                prefix: Vec::new(),
            },
        )
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cli_run_captures_output_and_surfaces_exit_code() {
        let backend = sh_backend("sh");
        let output = backend
            .run(&["-c".to_string(), "printf hello".to_string()])
            .await
            .expect("echo 必须执行成功");
        assert!(output.success);
        assert_eq!(output.stdout, "hello");

        let error = backend
            .run_checked(&["-c".to_string(), "echo boom >&2; exit 7".to_string()])
            .await
            .expect_err("非零退出码必须转成错误");
        assert!(matches!(error, crate::error::ContainerError::Cli { .. }));
        assert!(error.to_string().contains("boom"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cli_run_timeout_kills_child_promptly() {
        let backend = sh_backend("sh");
        let started = std::time::Instant::now();
        let error = backend
            .run_with_timeout(
                &["-c".to_string(), "sleep 5".to_string()],
                std::time::Duration::from_millis(150),
            )
            .await
            .expect_err("超时必须失败");
        assert!(matches!(error, crate::error::ContainerError::Cli { .. }));
        assert!(
            started.elapsed() < std::time::Duration::from_secs(3),
            "超时后必须尽快返回，实际耗时: {:?}",
            started.elapsed()
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn cli_run_cancellation_kills_child_promptly() {
        let backend = sh_backend("sh");
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let task_cancel = std::sync::Arc::clone(&cancel);
        let task = tokio::spawn(async move {
            backend
                .run_checked_cancellable(
                    &["-c".to_string(), "sleep 5".to_string()],
                    std::time::Duration::from_secs(30),
                    &task_cancel,
                )
                .await
        });
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        let result = task.await.expect("任务不能 panic");
        assert!(matches!(
            result,
            Err(crate::error::ContainerError::Cancelled)
        ));
    }
}
