//! 远端路径存在性探测适配器。
//!
//! 设计原理（Why）：检查点需要区分“确认不存在”和“探测失败”，因此单独封装
//! SFTP 状态码与命令回退路径，不让调用方根据错误文案猜测文件是否存在。

use transom_ai::ToolError;
use transom_core::ErrorCode;

use super::bridge::SshRemoteFsBridge;
use super::runtime_bridge::shell_quote;

pub(super) async fn remote_file_exists(
    bridge: &SshRemoteFsBridge,
    path: &str,
) -> Result<bool, ToolError> {
    if let Some(sftp) = &bridge.sftp {
        return match sftp.stat(path).await {
            Ok(_) => Ok(true),
            Err(err) if err.code == ErrorCode::SftpNotFound => Ok(false),
            Err(err) => Err(ToolError::ExecutionFailed {
                tool: "remote_fs",
                reason: err.to_string(),
            }),
        };
    }
    let out = bridge
        .remote
        .exec_command_capture(&format!("test -e {}", shell_quote(path)))
        .await
        .map_err(|err| ToolError::ExecutionFailed {
            tool: "remote_fs",
            reason: err.to_string(),
        })?;
    match out.exit_status {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        Some(code) => Err(ToolError::ExecutionFailed {
            tool: "remote_fs",
            reason: format!(
                "探测远端路径失败（{path}），退出码: {code}，stderr: {}",
                out.stderr
            ),
        }),
        None => Err(ToolError::ExecutionFailed {
            tool: "remote_fs",
            reason: format!("探测远端路径失败（{path}），未获取到退出码"),
        }),
    }
}
