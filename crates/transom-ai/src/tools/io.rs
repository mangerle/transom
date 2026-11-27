//! 远端 I/O 的统一截止时间与取消门。

use std::future::Future;
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use super::ToolError;

/// 远端文件、传输与运行时巡检的统一截止时间与取消上下文。
#[derive(Clone, Debug)]
pub struct ToolIoContext {
    pub timeout: Duration,
    pub cancel: CancellationToken,
}

impl ToolIoContext {
    /// 创建受超时与取消约束的 I/O 上下文。
    pub fn new(timeout: Duration, cancel: CancellationToken) -> Self {
        Self { timeout, cancel }
    }
}

/// 为任意返回 `ToolError` 的异步 I/O 添加统一 deadline 与取消门。
///
/// # Errors
/// 取消或超时分别返回 [`ToolError::Cancelled`] 与 [`ToolError::ExecutionTimeout`]；
/// 底层错误原样透传。
pub async fn with_io_guard<T, F>(
    tool: &'static str,
    context: &ToolIoContext,
    future: F,
) -> Result<T, ToolError>
where
    F: Future<Output = Result<T, ToolError>> + Send,
{
    let timeout_secs = context.timeout.as_secs().max(1);
    if context.cancel.is_cancelled() {
        return Err(ToolError::Cancelled);
    }
    tokio::pin!(future);
    tokio::select! {
        _ = context.cancel.cancelled() => Err(ToolError::Cancelled),
        result = tokio::time::timeout(context.timeout, &mut future) => {
            match result {
                Ok(result) => result,
                Err(_) => {
                    log::debug!("工具 {tool} 的 I/O 操作超过截止时间 {timeout_secs} 秒");
                    Err(ToolError::ExecutionTimeout { timeout_secs })
                }
            }
        }
    }
}
