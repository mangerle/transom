//! 统一目标路由（`target_resolver`）：按 `target` 区分本机（`"local"`）、已连接会话或自动触发连接。

use std::time::Duration;

use serde_json::Value;
use tokio::sync::oneshot;
use transom_ai::{HostProtection, ToolError};
use transom_core::{
    AiHostTargetDto, AuthMethod, Connection, ConnectionSecrets, OutboundProxy, SecretKey,
    SecretKind, Session, SshConnection, TabId,
};
use transom_net::{
    PasswordPrompter, SftpClient, SshRemote, SshRemoteConnectParams, connect_ssh_remote,
};

use super::ControllerAppActionBridge;
use crate::controller::Internal;
use crate::controller::ai_ops::mcp_bridge::{McpControllerSnapshot, McpLiveSession};

/// 判断工具参数是否明确指向本机执行（`target = "local"`，忽略大小写）。
pub(super) fn is_local_target(args: &Value) -> bool {
    args["target"]
        .as_str()
        .map(str::trim)
        .is_some_and(|s| s.eq_ignore_ascii_case("local"))
}

/// 从 `target`（或兼容字段 `session_id`）提取远端目标字符串。
pub(super) fn extract_remote_target(args: &Value) -> Option<&str> {
    args["target"]
        .as_str()
        .or_else(|| args["session_id"].as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("local"))
}

/// 按名称、`session_id` 或主机地址解析已连接会话；未连接时自动触发连接并等待就绪。
pub(super) async fn resolve_or_connect_target(
    bridge: &ControllerAppActionBridge,
    target: Option<&str>,
    timeout_secs: u64,
) -> Result<McpLiveSession, ToolError> {
    let snap = fetch_snapshot(bridge).await?;
    let wanted = target.map(str::trim).filter(|s| !s.is_empty());

    if wanted.is_none() || wanted == Some("active") {
        if let Some(tid) = bridge.deps.active_tab_id
            && let Some(found) = snap.sessions.into_iter().find(|s| s.meta.tab_id == tid)
        {
            return Ok(found);
        }
        return Err(ToolError::InvalidArguments {
            tool: "target_resolver",
            reason: "未指定目标服务器 target 且当前没有活动的已连接 SSH 标签页；请传入 target=\"local\" 操作本机，或传入服务器名（可先调用 session_catalog(action=\"list\") 查看）".to_owned(),
        });
    }

    let wanted_str = wanted.unwrap_or("");
    if let Some(live) = find_live_session(&snap.sessions, wanted_str) {
        return Ok(live);
    }

    let store = bridge.deps.session_store.clone();
    let wanted_owned = wanted_str.to_owned();
    let (session, ssh) = tokio::task::spawn_blocking(move || -> Result<(Session, SshConnection), ToolError> {
        let catalog = store.load().map_err(|e| ToolError::ExecutionFailed {
            tool: "target_resolver",
            reason: e.to_string(),
        })?;
        let ssh_sessions: Vec<_> = catalog
            .sessions
            .iter()
            .filter_map(|s| match &s.connection {
                Connection::Ssh(ssh) => Some((s, ssh)),
                _ => None,
            })
            .collect();

        if let Some((s, ssh)) = ssh_sessions
            .iter()
            .find(|(s, _)| s.id.to_string() == wanted_owned || s.name == wanted_owned)
        {
            return Ok(((*s).clone(), (*ssh).clone()));
        }
        let ci_matches: Vec<_> = ssh_sessions
            .iter()
            .filter(|(s, ssh)| {
                s.name.eq_ignore_ascii_case(&wanted_owned)
                    || ssh.host.eq_ignore_ascii_case(&wanted_owned)
            })
            .collect();
        match ci_matches.len() {
            1 => Ok((ci_matches[0].0.clone(), ci_matches[0].1.clone())),
            0 => Err(ToolError::InvalidArguments {
                tool: "target_resolver",
                reason: format!(
                    "未找到名为「{wanted_owned}」的已保存 SSH 会话，请先调用 session_catalog(action=\"list\") 确认名称或 session_id"
                ),
            }),
            _ => Err(ToolError::InvalidArguments {
                tool: "target_resolver",
                reason: format!(
                    "目标「{wanted_owned}」匹配到多个已保存 SSH 会话，请改用精确名称或 session_id"
                ),
            }),
        }
    })
    .await
    .map_err(|e| ToolError::ExecutionFailed {
        tool: "target_resolver",
        reason: e.to_string(),
    })??;

    let meta = AiHostTargetDto {
        tab_id: TabId::new(),
        session_id: session.id,
        host_name: session.name.clone(),
        login: format!("{}@{}:{}", ssh.username, ssh.host, ssh.port),
        platform: "linux".to_owned(),
        protection: ssh.protection,
    };
    if let Some(cached) = bridge
        .deps
        .headless_remotes
        .lock()
        .get(&session.id)
        .cloned()
    {
        return Ok(McpLiveSession {
            meta,
            remote: cached,
            sftp: None,
        });
    }

    let secret_store = bridge.deps.secret_store.clone();
    let owner = session.secret_owner();
    let auth = ssh.auth.clone();
    let credentials = tokio::task::spawn_blocking(move || {
        let mut creds = ConnectionSecrets::default();
        match auth {
            AuthMethod::Password { .. } => {
                if let Ok(Some(pwd)) =
                    secret_store.get(&SecretKey::new(owner, SecretKind::Password))
                {
                    creds.password = Some(pwd);
                }
            }
            AuthMethod::PrivateKey {
                save_passphrase: true,
                ..
            } => {
                if let Ok(Some(pass)) =
                    secret_store.get(&SecretKey::new(owner, SecretKind::KeyPassphrase))
                {
                    creds.key_passphrase = Some(pass);
                }
            }
            _ => {}
        }
        creds
    })
    .await
    .map_err(|e| ToolError::ExecutionFailed {
        tool: "target_resolver",
        reason: e.to_string(),
    })?;

    let prompter = std::sync::Arc::new(PasswordPrompter::new(
        credentials
            .password
            .as_deref()
            .map(String::as_str)
            .unwrap_or_default(),
    ));
    let params = SshRemoteConnectParams {
        connection: &ssh,
        jumps: &[],
        credentials,
        prompter,
        known_hosts: bridge.deps.known_hosts.clone(),
        proxy: OutboundProxy::default(),
        allow_sha1_publickey_auth: true,
    };
    let wait_secs = timeout_secs.clamp(10, 60);
    let remote = tokio::select! {
        _ = bridge.deps.cancel.cancelled() => return Err(ToolError::Cancelled),
        res = tokio::time::timeout(Duration::from_secs(wait_secs), connect_ssh_remote(params)) => {
            res.map_err(|_| ToolError::ExecutionFailed {
                tool: "target_resolver",
                reason: format!("后台静默连接「{wanted_str}」超时（{wait_secs}s），请检查网络连通性"),
            })?
            .map_err(|err| ToolError::ExecutionFailed {
                tool: "target_resolver",
                reason: format!("后台静默连接「{wanted_str}」失败（如需交互式输入密码，请先手动打开该会话或授权打开标签页）: {err}"),
            })?
        }
    };

    bridge
        .deps
        .headless_remotes
        .lock()
        .insert(session.id, remote.clone());
    Ok(McpLiveSession {
        meta,
        remote,
        sftp: None,
    })
}

fn find_live_session(sessions: &[McpLiveSession], wanted: &str) -> Option<McpLiveSession> {
    sessions
        .iter()
        .find(|s| s.meta.session_id.to_string() == wanted || s.meta.host_name == wanted)
        .or_else(|| {
            sessions
                .iter()
                .find(|s| s.meta.host_name.eq_ignore_ascii_case(wanted))
        })
        .cloned()
}

async fn fetch_snapshot(
    bridge: &ControllerAppActionBridge,
) -> Result<McpControllerSnapshot, ToolError> {
    let (tx, rx) = oneshot::channel();
    bridge
        .deps
        .internal_tx
        .send_async(Internal::McpSnapshot(tx))
        .await
        .map_err(|_| ToolError::ExecutionFailed {
            tool: "target_resolver",
            reason: "Transom 控制平面未就绪".to_owned(),
        })?;
    rx.await.map_err(|_| ToolError::ExecutionFailed {
        tool: "target_resolver",
        reason: "获取控制平面会话快照失败".to_owned(),
    })
}

/// 确保目标会话拥有可用 SFTP 客户端（未开启时按需通过现有 SSH 连接打开）。
pub(super) async fn ensure_sftp(session: &McpLiveSession) -> Result<SftpClient, ToolError> {
    if let Some(sftp) = &session.sftp {
        return Ok(sftp.clone());
    }
    SftpClient::open(&session.remote)
        .await
        .map_err(|e| ToolError::ExecutionFailed {
            tool: "target_resolver",
            reason: format!("打开远端 SFTP 通道失败: {e}"),
        })
}

/// 只读主机保护兜底拦截。
pub(super) fn guard_readonly(session: &McpLiveSession, is_write: bool) -> Result<(), ToolError> {
    if is_write && session.meta.protection == HostProtection::ReadOnly {
        return Err(ToolError::ExecutionFailed {
            tool: "target_resolver",
            reason: format!(
                "主机「{}」当前为只读保护模式（ReadOnly），已拦截写入或变更操作",
                session.meta.host_name
            ),
        });
    }
    Ok(())
}

pub(super) async fn ensure_remote_dir(remote: &SshRemote, dir: &str) -> Result<(), ToolError> {
    let quoted = shell_quote(dir);
    let out = remote
        .exec_command_capture(&format!("mkdir -p {quoted}"))
        .await
        .map_err(|e| ToolError::ExecutionFailed {
            tool: "target_resolver",
            reason: e.to_string(),
        })?;
    if out.exit_status == Some(0) {
        Ok(())
    } else {
        Err(ToolError::ExecutionFailed {
            tool: "target_resolver",
            reason: out.stderr,
        })
    }
}

pub(super) fn shell_quote(raw: &str) -> String {
    let escaped = raw.replace('\'', "'\"'\"'");
    format!("'{escaped}'")
}
