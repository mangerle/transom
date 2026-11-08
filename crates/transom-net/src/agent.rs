//! 本地 SSH Agent 连接解析。
//!
//! 设计原理（Why）：认证与 Agent 转发需要完全相同的本地端点选择逻辑。
//! Unix 使用 `SSH_AUTH_SOCK`；Windows 既可能是 OpenSSH Agent 命名管道，
//! 也可能是 Pageant。把候选顺序、超时和错误收敛到这里，认证与转发不再
//! 各自维护一套平台分支。
//!
//! 代价与局限：Windows 的 Pageant 通过窗口消息适配成异步流，连接超时由
//! 统一上限控制；若用户同时运行多个 Agent，按 OpenSSH 管道优先。

use russh::keys::agent::client::AgentStream as RusshAgentStream;
use std::time::Duration;
use transom_core::AppError;

use crate::error::NetError;

/// 单个 Agent 端点的连接超时。
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);

/// 可作为 SSH Agent 双向字节流的对象。
pub(crate) type AgentStream = Box<dyn RusshAgentStream + Send + Unpin>;

/// 连接当前用户的 SSH Agent。
///
/// 成功返回原始双向流，认证侧可包装为 russh `AgentClient`，转发侧可直接
/// `copy_bidirectional`。失败返回带候选端点的中文认证错误。
pub(crate) async fn connect_agent_stream() -> Result<AgentStream, AppError> {
    #[cfg(unix)]
    {
        connect_unix().await
    }
    #[cfg(windows)]
    {
        connect_windows().await
    }
    #[cfg(not(any(unix, windows)))]
    {
        Err(NetError::Auth {
            reason: "当前平台没有可用的 SSH Agent 连接实现".to_string(),
        }
        .into())
    }
}

#[cfg(unix)]
async fn connect_unix() -> Result<AgentStream, AppError> {
    use russh::keys::agent::client::AgentClient;

    let path = std::env::var("SSH_AUTH_SOCK")
        .ok()
        .filter(|path| !path.trim().is_empty())
        .ok_or_else(|| NetError::Auth {
            reason: "未检测到有效的 SSH_AUTH_SOCK 环境变量".to_string(),
        })?;
    let client = tokio::time::timeout(CONNECT_TIMEOUT, AgentClient::connect_uds(&path))
        .await
        .map_err(|_| NetError::Auth {
            reason: format!(
                "连接 SSH Agent 超时，套接字: {path}，超时: {} 秒",
                CONNECT_TIMEOUT.as_secs()
            ),
        })?
        .map_err(|error| NetError::Auth {
            reason: format!("连接 SSH Agent 失败，套接字: {path}，原因: {error}"),
        })?;
    Ok(client.into_inner())
}

#[cfg(windows)]
async fn connect_windows() -> Result<AgentStream, AppError> {
    use russh::keys::agent::client::AgentClient;

    const OPENSSH_AGENT_PIPE: &str = r"\\.\pipe\openssh-ssh-agent";

    let mut candidates: Vec<String> = Vec::with_capacity(3);
    if let Ok(path) = std::env::var("SSH_AUTH_SOCK")
        && !path.trim().is_empty()
    {
        candidates.push(path);
    }
    if !candidates.iter().any(|path| path == OPENSSH_AGENT_PIPE) {
        candidates.push(OPENSSH_AGENT_PIPE.to_string());
    }

    let mut errors: Vec<String> = Vec::with_capacity(candidates.len() + 1);
    for path in candidates {
        match tokio::time::timeout(CONNECT_TIMEOUT, AgentClient::connect_named_pipe(&path)).await {
            Ok(Ok(client)) => {
                log::debug!("已连接 Windows OpenSSH Agent 命名管道: {path}");
                return Ok(client.into_inner());
            }
            Ok(Err(error)) => errors.push(format!("管道 {path}: {error}")),
            Err(_) => errors.push(format!(
                "管道 {path}: 连接超时 {} 秒",
                CONNECT_TIMEOUT.as_secs()
            )),
        }
    }
    match tokio::time::timeout(CONNECT_TIMEOUT, AgentClient::connect_pageant()).await {
        Ok(Ok(client)) => {
            log::debug!("已连接 Pageant SSH Agent");
            Ok(client.into_inner())
        }
        Ok(Err(error)) => {
            errors.push(format!("Pageant: {error}"));
            Err(NetError::Auth {
                reason: format!(
                    "未找到可用的 Windows SSH Agent，尝试结果: {}",
                    errors.join("；")
                ),
            }
            .into())
        }
        Err(_) => {
            errors.push(format!(
                "Pageant: 连接超时 {} 秒",
                CONNECT_TIMEOUT.as_secs()
            ));
            Err(NetError::Auth {
                reason: format!(
                    "未找到可用的 Windows SSH Agent，尝试结果: {}",
                    errors.join("；")
                ),
            }
            .into())
        }
    }
}
