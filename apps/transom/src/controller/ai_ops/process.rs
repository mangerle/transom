//! AI 控制平面外部进程的有界输出捕获。

use std::borrow::Cow;
use std::io;
use std::process::{ExitStatus, Stdio};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt};
use tokio_util::sync::CancellationToken;
use transom_ai::ToolError;

/// 外部进程单次捕获的最大输出字节数。
pub(in crate::controller) const MAX_PROCESS_CAPTURE_BYTES: usize = 4 * 1024 * 1024;

/// 有界捕获后的进程结果。
pub(in crate::controller) struct CapturedProcessOutput {
    pub(in crate::controller) status: ExitStatus,
    pub(in crate::controller) stdout: Vec<u8>,
    pub(in crate::controller) stderr: Vec<u8>,
    pub(in crate::controller) stdout_truncated: bool,
    pub(in crate::controller) stderr_truncated: bool,
}

impl CapturedProcessOutput {
    /// 以 UTF-8 替换非法字节后的标准输出文本。
    pub(in crate::controller) fn stdout_lossy(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(&self.stdout)
    }

    /// 以 UTF-8 替换非法字节后的标准错误文本。
    pub(in crate::controller) fn stderr_lossy(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(&self.stderr)
    }
}

/// 启动外部进程并捕获受控大小的标准输出与标准错误。
///
/// 该函数统一强制 stdin 为空、stdout/stderr 管道化、子进程随 Future 取消而终止，
/// 并在正常完成时等待子进程退出。读取达到上限后仍持续排空管道，避免子进程因
/// 管道背压阻塞；返回值通过截断标志明确告知调用方。
///
/// # Errors
/// 启动失败、管道缺失、读取失败、超时或取消时返回 [`ToolError`]。
pub(in crate::controller) async fn capture_command_output(
    command: &mut tokio::process::Command,
    tool: &'static str,
    timeout: Duration,
    cancel: CancellationToken,
    max_bytes: usize,
) -> Result<CapturedProcessOutput, ToolError> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let mut child = command
        .spawn()
        .map_err(|error| ToolError::ExecutionFailed {
            tool,
            reason: format!("启动外部进程失败: {error}"),
        })?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| ToolError::ExecutionFailed {
            tool,
            reason: "外部进程标准输出管道不可用".to_owned(),
        })?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| ToolError::ExecutionFailed {
            tool,
            reason: "外部进程标准错误管道不可用".to_owned(),
        })?;
    let max_bytes = max_bytes.max(1);

    let capture = async {
        let (stdout_result, stderr_result, status_result) = tokio::join!(
            read_limited(&mut stdout, max_bytes),
            read_limited(&mut stderr, max_bytes),
            child.wait(),
        );
        let status = status_result.map_err(|error| ToolError::ExecutionFailed {
            tool,
            reason: format!("等待外部进程退出失败: {error}"),
        })?;
        let (stdout, stdout_truncated) =
            stdout_result.map_err(|error| ToolError::ExecutionFailed {
                tool,
                reason: format!("读取外部进程标准输出失败: {error}"),
            })?;
        let (stderr, stderr_truncated) =
            stderr_result.map_err(|error| ToolError::ExecutionFailed {
                tool,
                reason: format!("读取外部进程标准错误失败: {error}"),
            })?;
        Ok(CapturedProcessOutput {
            status,
            stdout,
            stderr,
            stdout_truncated,
            stderr_truncated,
        })
    };

    tokio::select! {
        _ = cancel.cancelled() => Err(ToolError::Cancelled),
        result = tokio::time::timeout(timeout, capture) => {
            result.map_err(|_| ToolError::ExecutionTimeout {
                timeout_secs: timeout.as_secs(),
            })?
        }
    }
}

async fn read_limited<R>(reader: &mut R, max_bytes: usize) -> io::Result<(Vec<u8>, bool)>
where
    R: AsyncRead + Unpin,
{
    let mut output = Vec::with_capacity(max_bytes.min(64 * 1024));
    let mut buffer = [0u8; 8 * 1024];
    let mut truncated = false;
    loop {
        let read = reader.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        let available = max_bytes.saturating_sub(output.len());
        let take = available.min(read);
        output.extend_from_slice(&buffer[..take]);
        if take < read {
            truncated = true;
        }
    }
    Ok((output, truncated))
}

/// 将一段文本追加到有界字符串，返回是否发生截断。
pub(in crate::controller) fn append_bounded_text(
    target: &mut String,
    chunk: &str,
    max_bytes: usize,
) -> bool {
    let available = max_bytes.saturating_sub(target.len());
    if available == 0 {
        return !chunk.is_empty();
    }
    let mut take = available.min(chunk.len());
    while take > 0 && !chunk.is_char_boundary(take) {
        take -= 1;
    }
    target.push_str(&chunk[..take]);
    take < chunk.len()
}

#[cfg(test)]
mod tests {
    use super::append_bounded_text;

    #[test]
    fn appends_text_without_splitting_utf8_characters() {
        let mut output = String::new();
        assert!(append_bounded_text(&mut output, "abc中文", 7));
        assert_eq!(output, "abc中");

        let mut exact = String::new();
        assert!(!append_bounded_text(&mut exact, "abc中", 6));
        assert_eq!(exact, "abc中");
    }

    #[test]
    fn reports_truncation_after_limit() {
        let mut output = String::from("12345");
        assert!(append_bounded_text(&mut output, "6789", 6));
        assert_eq!(output, "123456");
    }
}
