//! 反向 MCP（Model Context Protocol）服务端实现（支持 stdio 与本地 Unix Socket）。
//!
//! 设计原理（Why）：
//! - 允许外部编程 Agent（如 Claude Code、Cursor、Codex）通过标准 MCP JSON-RPC 2.0 协议复用 Transom 已建立的 SSH/SFTP 会话；
//! - 服务端本身只暴露协议解析与传输循环，所有工具调用与策略审批均委托给 [`McpServerBackend`]，
//!   由控制平面强制经过 `HostProtection`、`detect_blocking_command` 与 UI 审批门，外部 Agent 无法绕过安全护栏；
//! - `list_sessions` 仅返回脱敏后的主机元数据，严禁暴露密码或私钥。

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncBufRead, AsyncWrite, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

use transom_platform::private_fs;

use crate::error::AiError;
use crate::policy::HostProtection;
use crate::protocol_lines::{MAX_PROTOCOL_LINE_BYTES, read_bounded_line};

/// MCP 协议版本。
pub const MCP_PROTOCOL_VERSION: &str = "2024-11-05";

/// 暴露给外部 MCP 客户端的脱敏会话摘要。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpSessionInfo {
    pub session_id: String,
    pub host_name: String,
    pub login: String,
    pub platform: String,
    pub protection: HostProtection,
}

/// 外部 MCP 调用的实际后端抽象（由控制平面实现）。
#[async_trait]
pub trait McpServerBackend: Send + Sync {
    async fn list_sessions(&self) -> Vec<McpSessionInfo>;
    async fn call_tool(&self, name: &str, arguments: Value) -> Result<String, String>;
}

/// Transom 本地 MCP 服务端。
pub struct McpServer {
    backend: Arc<dyn McpServerBackend>,
}

impl McpServer {
    pub fn new(backend: Arc<dyn McpServerBackend>) -> Self {
        Self { backend }
    }

    /// 处理单行 JSON-RPC 2.0 请求，通知类报文返回 `None`。
    pub async fn handle_jsonrpc_line(&self, line: &str) -> Option<String> {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return None;
        }
        let req: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(err) => {
                let resp = jsonrpc_error(Value::Null, -32700, format!("JSON 解析失败: {err}"));
                return serde_json::to_string(&resp).ok();
            }
        };

        let id = req.get("id").cloned();
        let Some(method) = req.get("method").and_then(Value::as_str) else {
            return id.and_then(|req_id| {
                serde_json::to_string(&jsonrpc_error(
                    req_id,
                    -32600,
                    "缺少 method 字段".to_owned(),
                ))
                .ok()
            });
        };

        // 无 id 的 JSON-RPC notification 不回包。
        if method == "notifications/initialized" || id.is_none() {
            return None;
        }
        let req_id = id.unwrap_or(Value::Null);
        let params = req.get("params").cloned().unwrap_or_else(|| json!({}));

        let resp = match method {
            "initialize" => jsonrpc_ok(
                req_id,
                json!({
                    "protocolVersion": MCP_PROTOCOL_VERSION,
                    "capabilities": { "tools": {} },
                    "serverInfo": {
                        "name": "transom-mcp",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                }),
            ),
            "ping" => jsonrpc_ok(req_id, json!({})),
            "tools/list" => jsonrpc_ok(
                req_id,
                json!({ "tools": crate::mcp_tools::tool_definitions() }),
            ),
            "tools/call" => self.handle_tool_call(req_id, params).await,
            other => jsonrpc_error(req_id, -32601, format!("不支持的 MCP 方法: {other}")),
        };
        serde_json::to_string(&resp).ok()
    }

    async fn handle_tool_call(&self, req_id: Value, params: Value) -> Value {
        let Some(name) = params.get("name").and_then(Value::as_str) else {
            return jsonrpc_error(req_id, -32602, "缺少工具名称 name".to_owned());
        };
        let args = params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));

        if name == "list_sessions" {
            let sessions = self.backend.list_sessions().await;
            let text = serde_json::to_string_pretty(&sessions).unwrap_or_else(|_| "[]".to_owned());
            return jsonrpc_ok(
                req_id,
                json!({
                    "content": [{ "type": "text", "text": text }],
                    "isError": false
                }),
            );
        }

        match self.backend.call_tool(name, args).await {
            Ok(text) => jsonrpc_ok(
                req_id,
                json!({
                    "content": [{ "type": "text", "text": text }],
                    "isError": false
                }),
            ),
            Err(err_text) => jsonrpc_ok(
                req_id,
                json!({
                    "content": [{ "type": "text", "text": err_text }],
                    "isError": true
                }),
            ),
        }
    }
}

/// 在任意异步字节流（如 stdio 或 Unix Socket 连接）上运行 MCP 服务端循环。
///
/// # Errors
/// 底层流读取或写入失败时返回 [`AiError::McpFailed`]。
pub async fn serve_stream<R, W>(
    server: Arc<McpServer>,
    reader: R,
    mut writer: W,
    cancel: CancellationToken,
) -> Result<(), AiError>
where
    R: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut reader = reader;
    loop {
        tokio::select! {
            _ = cancel.cancelled() => return Ok(()),
            res = read_bounded_line(&mut reader, MAX_PROTOCOL_LINE_BYTES) => {
                let line = res.map_err(|err| AiError::McpFailed {
                    server: "transom-mcp".to_owned(),
                    reason: err.to_string(),
                })?;
                let Some(line) = line else {
                    return Ok(());
                };
                if let Some(reply) = server.handle_jsonrpc_line(&line).await {
                    writer.write_all(reply.as_bytes()).await.map_err(|err| AiError::McpFailed {
                        server: "transom-mcp".to_owned(),
                        reason: err.to_string(),
                    })?;
                    writer.write_all(b"\n").await.map_err(|err| AiError::McpFailed {
                        server: "transom-mcp".to_owned(),
                        reason: err.to_string(),
                    })?;
                    writer.flush().await.map_err(|err| AiError::McpFailed {
                        server: "transom-mcp".to_owned(),
                        reason: err.to_string(),
                    })?;
                }
            }
        }
    }
}

/// 在本地 Unix Domain Socket 上启动 MCP 服务端（仅限当前用户 `0o600` 访问）。
///
/// # Errors
/// 绑定或创建目录失败时返回 [`AiError::McpFailed`]。
#[cfg(unix)]
pub async fn serve_local_socket(
    server: Arc<McpServer>,
    socket_path: &Path,
    cancel: CancellationToken,
) -> Result<(), AiError> {
    use std::os::unix::fs::{FileTypeExt as _, PermissionsExt as _};
    use tokio::io::BufReader;

    if let Some(parent) = socket_path.parent() {
        private_fs::ensure_private_dir(parent).map_err(|err| AiError::McpFailed {
            server: "transom-mcp".to_owned(),
            reason: format!("创建 MCP Socket 目录失败（{}）: {err}", parent.display()),
        })?;
    }
    if let Ok(metadata) = std::fs::symlink_metadata(socket_path) {
        if !metadata.file_type().is_socket() {
            return Err(AiError::McpFailed {
                server: "transom-mcp".to_owned(),
                reason: format!(
                    "MCP Socket 路径已存在且不是 Socket: {}",
                    socket_path.display()
                ),
            });
        }
        std::fs::remove_file(socket_path).map_err(|err| AiError::McpFailed {
            server: "transom-mcp".to_owned(),
            reason: format!("清理旧 MCP Socket 失败（{}）: {err}", socket_path.display()),
        })?;
    }

    let listener =
        tokio::net::UnixListener::bind(socket_path).map_err(|err| AiError::McpFailed {
            server: "transom-mcp".to_owned(),
            reason: format!(
                "绑定本地 MCP Socket 失败（{}）: {err}",
                socket_path.display()
            ),
        })?;
    if let Err(err) = std::fs::set_permissions(socket_path, std::fs::Permissions::from_mode(0o600))
    {
        let _ = std::fs::remove_file(socket_path);
        return Err(AiError::McpFailed {
            server: "transom-mcp".to_owned(),
            reason: format!(
                "收紧 MCP Socket 权限失败（{}）: {err}",
                socket_path.display()
            ),
        });
    }
    log::info!("Transom 本地 MCP Socket 已就绪: {}", socket_path.display());

    let mut conn_tasks = tokio::task::JoinSet::new();
    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, _addr)) => {
                        let srv = server.clone();
                        let child_cancel = cancel.child_token();
                        conn_tasks.spawn(async move {
                            let (read_half, write_half) = stream.into_split();
                            let _ = serve_stream(srv, BufReader::new(read_half), write_half, child_cancel).await;
                        });
                    }
                    Err(err) => {
                        log::warn!("接收本地 MCP 连接失败: {err}");
                    }
                }
            }
        }
    }

    let _ = tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while conn_tasks.join_next().await.is_some() {}
    })
    .await;
    let _ = std::fs::remove_file(socket_path);
    Ok(())
}

/// 非 Unix 平台占位实现。
///
/// # Errors
/// 当前版本仅支持 Unix Domain Socket 本地暴露。
#[cfg(not(unix))]
pub async fn serve_local_socket(
    _server: Arc<McpServer>,
    _socket_path: &Path,
    _cancel: CancellationToken,
) -> Result<(), AiError> {
    Err(AiError::McpFailed {
        server: "transom-mcp".to_owned(),
        reason: "当前平台暂不支持 Unix Domain Socket 本地 MCP 服务".to_owned(),
    })
}

fn jsonrpc_ok(id: Value, result: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result
    })
}

fn jsonrpc_error(id: Value, code: i64, message: String) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": code,
            "message": message
        }
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use serde_json::Value;

    use super::{McpServer, McpServerBackend, McpSessionInfo};
    use crate::policy::HostProtection;

    struct FakeBackend;

    #[async_trait]
    impl McpServerBackend for FakeBackend {
        async fn list_sessions(&self) -> Vec<McpSessionInfo> {
            vec![McpSessionInfo {
                session_id: "s-1".to_owned(),
                host_name: "api-01".to_owned(),
                login: "ops@10.0.0.1:22".to_owned(),
                platform: "linux".to_owned(),
                protection: HostProtection::Normal,
            }]
        }

        async fn call_tool(&self, name: &str, arguments: Value) -> Result<String, String> {
            if name == "remote_exec" {
                let cmd = arguments["command"].as_str().unwrap_or("");
                Ok(format!("退出码: 0\nran:{cmd}"))
            } else {
                Err(format!("未知工具: {name}"))
            }
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn local_socket_is_owner_only() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = tempfile::tempdir().expect("创建临时目录失败");
        let socket_path = dir.path().join("mcp.sock");
        let server = Arc::new(McpServer::new(Arc::new(FakeBackend)));
        let cancel = tokio_util::sync::CancellationToken::new();
        let task_socket = socket_path.clone();
        let task_cancel = cancel.clone();
        let serve = tokio::spawn(async move {
            super::serve_local_socket(server, &task_socket, task_cancel).await
        });
        for _ in 0..100 {
            if socket_path.exists() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let mode = std::fs::metadata(&socket_path)
            .expect("读取 Socket 属性失败")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
        cancel.cancel();
        serve
            .await
            .expect("MCP 服务任务异常")
            .expect("MCP 服务失败");
    }

    #[tokio::test]
    async fn handles_initialize_tools_list_and_call() {
        let srv = McpServer::new(Arc::new(FakeBackend));
        let init = srv
            .handle_jsonrpc_line(r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#)
            .await
            .expect("initialize 必须返回响应");
        assert!(init.contains("2024-11-05"));

        assert!(
            srv.handle_jsonrpc_line(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
                .await
                .is_none()
        );

        let list = srv
            .handle_jsonrpc_line(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#)
            .await
            .expect("tools/list 必须返回响应");
        assert!(list.contains("remote_exec_bulk"));
        assert!(!list.contains("compare_hosts"));
        assert!(list.contains("session_catalog"));
        assert!(list.contains("workspace_control"));
        assert!(list.contains("container_workspace"));
        assert!(list.contains("git_workspace"));
        assert!(list.contains("ai_memory_skills"));

        let sessions = srv
            .handle_jsonrpc_line(
                r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"list_sessions","arguments":{}}}"#,
            )
            .await
            .expect("list_sessions 必须返回");
        assert!(sessions.contains("api-01"));

        let call_ok = srv
            .handle_jsonrpc_line(
                r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"remote_exec","arguments":{"session_id":"s-1","command":"uptime"}}}"#,
            )
            .await
            .expect("remote_exec 必须返回");
        assert!(call_ok.contains("ran:uptime"));
    }
}
