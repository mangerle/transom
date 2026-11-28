//! 领域 1：会话目录、分组、known_hosts 与公钥部署。

use std::path::PathBuf;

use serde_json::{Value, json};
use transom_ai::ToolError;
use transom_core::{
    AuthMethod, Connection, FolderId, HostProtection, LocalConnection, ProxyJumpConfig,
    RdpConnection, SerialConnection, Session, SessionFolder, SessionId, SshConnection,
};

use super::ControllerAppActionBridge;
use super::file_io::read_text_prefix;

pub(super) async fn handle(
    bridge: &ControllerAppActionBridge,
    action: &str,
    args: &Value,
) -> Result<String, ToolError> {
    match action {
        "list" => list_catalog(bridge).await,
        "create" => create_session(bridge, args).await,
        "update" => update_session(bridge, args).await,
        "delete" => delete_session(bridge, args).await,
        "create_folder" => create_folder(bridge, args).await,
        "delete_folder" => delete_folder(bridge, args).await,
        "list_known_hosts" => list_known_hosts(bridge).await,
        "remove_known_host" => remove_known_host(bridge, args).await,
        "deploy_public_key" => deploy_public_key(bridge, args).await,
        other => Err(ToolError::InvalidArguments {
            tool: "session_catalog",
            reason: format!("未知动作: {other}"),
        }),
    }
}

async fn list_catalog(bridge: &ControllerAppActionBridge) -> Result<String, ToolError> {
    let store = bridge.deps.stores.session_store.clone();
    let catalog = tokio::task::spawn_blocking(move || store.load())
        .await
        .map_err(|e| ToolError::ExecutionFailed {
            tool: "session_catalog",
            reason: e.to_string(),
        })?
        .map_err(|e| ToolError::ExecutionFailed {
            tool: "session_catalog",
            reason: e.to_string(),
        })?;

    let folders: Vec<Value> = catalog
        .folders
        .iter()
        .map(|f| {
            json!({
                "id": f.id.to_string(),
                "name": f.name,
                "parent_id": f.parent.map(|id| id.to_string()),
            })
        })
        .collect();

    // 脱敏输出：只返回连接参数摘要与认证方式名称，严禁包含密码、私钥或 Passphrase。
    let sessions: Vec<Value> = catalog.sessions.iter().map(summarize_session).collect();

    serde_json::to_string_pretty(&json!({ "folders": folders, "sessions": sessions })).map_err(
        |e| ToolError::ExecutionFailed {
            tool: "session_catalog",
            reason: e.to_string(),
        },
    )
}

fn summarize_session(s: &Session) -> Value {
    match &s.connection {
        Connection::Ssh(ssh) => json!({
            "id": s.id.to_string(),
            "name": s.name,
            "kind": "ssh",
            "folder_id": s.folder_id.map(|id| id.to_string()),
            "host": ssh.host,
            "port": ssh.port,
            "username": ssh.username,
            "auth": match &ssh.auth {
                AuthMethod::Password { .. } => "password",
                AuthMethod::PrivateKey { .. } => "private_key",
                AuthMethod::Agent => "agent",
                _ => "other",
            },
            "protection": format!("{:?}", ssh.protection),
            "jump_session_id": ssh.proxy_jump.as_ref().map(|j| j.jump_session_id.to_string()),
        }),
        Connection::Local(_) => json!({
            "id": s.id.to_string(),
            "name": s.name,
            "kind": "local",
            "folder_id": s.folder_id.map(|id| id.to_string()),
        }),
        Connection::Rdp(rdp) => json!({
            "id": s.id.to_string(),
            "name": s.name,
            "kind": "rdp",
            "host": rdp.host,
            "port": rdp.port,
            "username": rdp.username,
            "folder_id": s.folder_id.map(|id| id.to_string()),
        }),
        Connection::Serial(ser) => json!({
            "id": s.id.to_string(),
            "name": s.name,
            "kind": "serial",
            "port": ser.port,
            "baud_rate": ser.baud_rate,
            "folder_id": s.folder_id.map(|id| id.to_string()),
        }),
        _ => json!({
            "id": s.id.to_string(),
            "name": s.name,
            "kind": "other",
            "folder_id": s.folder_id.map(|id| id.to_string()),
        }),
    }
}

async fn create_session(
    bridge: &ControllerAppActionBridge,
    args: &Value,
) -> Result<String, ToolError> {
    let name = args["name"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ToolError::InvalidArguments {
            tool: "session_catalog",
            reason: "创建会话必须提供非空 name".to_owned(),
        })?
        .to_owned();
    let kind = args["kind"].as_str().unwrap_or("ssh");
    let conn = build_connection(kind, args)?;
    let mut session = Session::new(name.clone(), conn);
    if let Some(fid) = args["folder_id"].as_str().and_then(parse_folder_id) {
        session.folder_id = Some(fid);
    }
    let created_id = session.id.to_string();

    mutate_catalog_and_reload(bridge, move |catalog| {
        catalog.sessions.push(session);
        Ok(())
    })
    .await?;
    Ok(format!("已创建会话「{name}」，session_id: {created_id}"))
}

async fn mutate_catalog_and_reload(
    bridge: &ControllerAppActionBridge,
    mutator: impl FnOnce(&mut transom_core::SessionCatalog) -> Result<(), transom_core::AppError>
    + Send
    + 'static,
) -> Result<(), ToolError> {
    let store = bridge.deps.stores.session_store.clone();
    let catalog = tokio::task::spawn_blocking(move || {
        let mut catalog = store.load()?;
        mutator(&mut catalog)?;
        store.save(&catalog)?;
        Ok::<_, transom_core::AppError>(catalog)
    })
    .await
    .map_err(|e| ToolError::ExecutionFailed {
        tool: "session_catalog",
        reason: e.to_string(),
    })?
    .map_err(|e| ToolError::ExecutionFailed {
        tool: "session_catalog",
        reason: e.to_string(),
    })?;

    let payload = serde_json::json!({ "catalog": catalog });
    let _ = bridge
        .request_ui_action("session_catalog", "reload_catalog", &payload)
        .await;
    Ok(())
}

fn build_connection(kind: &str, args: &Value) -> Result<Connection, ToolError> {
    match kind {
        "local" => Ok(Connection::Local(LocalConnection::new())),
        "rdp" => {
            let host = req_str(args, "host")?.to_owned();
            let username = args["username"]
                .as_str()
                .unwrap_or("Administrator")
                .to_owned();
            let mut rdp = RdpConnection::new(host, username);
            if let Some(port) = args["port"].as_u64() {
                rdp.port = port as u16;
            }
            Ok(Connection::Rdp(rdp))
        }
        "serial" => {
            let port_name = req_str(args, "host")?.to_owned();
            let baud = args["port"].as_u64().unwrap_or(115_200) as u32;
            let ser = SerialConnection::new(port_name, baud);
            Ok(Connection::Serial(ser))
        }
        _ => {
            let host = req_str(args, "host")?.to_owned();
            let username = req_str(args, "username")?.to_owned();
            let mut ssh = SshConnection::new(host, username);
            if let Some(port) = args["port"].as_u64() {
                ssh.port = port as u16;
            }
            if let Some(key_path) = args["private_key_path"].as_str().filter(|s| !s.is_empty()) {
                ssh.auth = AuthMethod::PrivateKey {
                    path: PathBuf::from(key_path),
                    save_passphrase: false,
                    cert_path: None,
                };
            }
            if let Some(prot) = args["protection"].as_str() {
                ssh.protection = parse_protection(prot);
            }
            if let Some(jump_id) = args["jump_session_id"].as_str().and_then(parse_session_id) {
                ssh.proxy_jump = Some(ProxyJumpConfig {
                    jump_session_id: jump_id,
                });
            }
            Ok(Connection::Ssh(ssh))
        }
    }
}

async fn update_session(
    bridge: &ControllerAppActionBridge,
    args: &Value,
) -> Result<String, ToolError> {
    let sid = args["session_id"]
        .as_str()
        .and_then(parse_session_id)
        .ok_or_else(|| ToolError::InvalidArguments {
            tool: "session_catalog",
            reason: "update 必须提供有效的 session_id".to_owned(),
        })?;
    let args_cloned = args.clone();
    mutate_catalog_and_reload(bridge, move |catalog| {
        let session = catalog
            .sessions
            .iter_mut()
            .find(|s| s.id == sid)
            .ok_or_else(|| {
                transom_core::AppError::with_detail(
                    transom_core::ErrorCode::Configuration,
                    format!("未找到会话: {sid}"),
                )
            })?;
        if let Some(name) = args_cloned["name"]
            .as_str()
            .filter(|s| !s.trim().is_empty())
        {
            session.name = name.trim().to_owned();
        }
        if let Some(fid) = args_cloned["folder_id"].as_str() {
            session.folder_id = parse_folder_id(fid);
        }
        if let Connection::Ssh(ssh) = &mut session.connection {
            if let Some(host) = args_cloned["host"].as_str().filter(|s| !s.is_empty()) {
                ssh.host = host.to_owned();
            }
            if let Some(port) = args_cloned["port"].as_u64() {
                ssh.port = port as u16;
            }
            if let Some(user) = args_cloned["username"].as_str().filter(|s| !s.is_empty()) {
                ssh.username = user.to_owned();
            }
            if let Some(prot) = args_cloned["protection"].as_str() {
                ssh.protection = parse_protection(prot);
            }
            if let Some(key_path) = args_cloned["private_key_path"].as_str() {
                ssh.auth = if key_path.is_empty() {
                    AuthMethod::Agent
                } else {
                    AuthMethod::PrivateKey {
                        path: PathBuf::from(key_path),
                        save_passphrase: false,
                        cert_path: None,
                    }
                };
            }
        }
        session.touch();
        Ok(())
    })
    .await?;
    Ok(format!("已更新会话配置: {sid}"))
}

async fn delete_session(
    bridge: &ControllerAppActionBridge,
    args: &Value,
) -> Result<String, ToolError> {
    let sid = args["session_id"]
        .as_str()
        .and_then(parse_session_id)
        .ok_or_else(|| ToolError::InvalidArguments {
            tool: "session_catalog",
            reason: "delete 必须提供有效的 session_id".to_owned(),
        })?;
    mutate_catalog_and_reload(bridge, move |catalog| {
        let before = catalog.sessions.len();
        catalog.sessions.retain(|s| s.id != sid);
        if catalog.sessions.len() == before {
            return Err(transom_core::AppError::with_detail(
                transom_core::ErrorCode::Configuration,
                format!("未找到会话: {sid}"),
            ));
        }
        Ok(())
    })
    .await?;
    Ok(format!("已删除会话: {sid}"))
}

async fn create_folder(
    bridge: &ControllerAppActionBridge,
    args: &Value,
) -> Result<String, ToolError> {
    let name = req_str(args, "name")?.to_owned();
    let parent = args["parent_id"].as_str().and_then(parse_folder_id);
    let folder = SessionFolder::new(name.clone(), parent);
    let fid = folder.id.to_string();

    mutate_catalog_and_reload(bridge, move |catalog| {
        catalog.folders.push(folder);
        Ok(())
    })
    .await?;
    Ok(format!("已创建分组目录「{name}」，folder_id: {fid}"))
}

async fn delete_folder(
    bridge: &ControllerAppActionBridge,
    args: &Value,
) -> Result<String, ToolError> {
    let fid = args["folder_id"]
        .as_str()
        .and_then(parse_folder_id)
        .ok_or_else(|| ToolError::InvalidArguments {
            tool: "session_catalog",
            reason: "delete_folder 必须提供有效的 folder_id".to_owned(),
        })?;
    mutate_catalog_and_reload(bridge, move |catalog| {
        catalog.delete_folder(fid).map(|_| ()).map_err(|e| {
            transom_core::AppError::with_detail(
                transom_core::ErrorCode::Configuration,
                e.to_string(),
            )
        })
    })
    .await?;
    Ok(format!("已删除分组目录: {fid}"))
}

async fn list_known_hosts(bridge: &ControllerAppActionBridge) -> Result<String, ToolError> {
    let kh = bridge.deps.stores.known_hosts.clone();
    let records = tokio::task::spawn_blocking(move || kh.list())
        .await
        .map_err(|e| ToolError::ExecutionFailed {
            tool: "session_catalog",
            reason: e.to_string(),
        })?;
    serde_json::to_string_pretty(&records).map_err(|e| ToolError::ExecutionFailed {
        tool: "session_catalog",
        reason: e.to_string(),
    })
}

async fn remove_known_host(
    bridge: &ControllerAppActionBridge,
    args: &Value,
) -> Result<String, ToolError> {
    let host = req_str(args, "host")?.to_owned();
    let port = args["port"].as_u64().unwrap_or(22) as u16;
    let kh = bridge.deps.stores.known_hosts.clone();
    tokio::task::spawn_blocking(move || kh.remove(&host, port))
        .await
        .map_err(|e| ToolError::ExecutionFailed {
            tool: "session_catalog",
            reason: e.to_string(),
        })?
        .map_err(|e| ToolError::ExecutionFailed {
            tool: "session_catalog",
            reason: e.to_string(),
        })?;
    Ok("已从 known_hosts 移除对应主机指纹记录".to_owned())
}

async fn deploy_public_key(
    bridge: &ControllerAppActionBridge,
    args: &Value,
) -> Result<String, ToolError> {
    let Some(remote) = &bridge.deps.target.active_remote else {
        return Err(ToolError::NoActiveSession);
    };
    let pub_path = args["public_key_path"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ToolError::InvalidArguments {
            tool: "session_catalog",
            reason: "deploy_public_key 必须提供本机公钥路径 public_key_path".to_owned(),
        })?;
    let (key_text, truncated) =
        read_text_prefix(std::path::Path::new(pub_path), 64 * 1024, "session_catalog").await?;
    if truncated {
        return Err(ToolError::InvalidArguments {
            tool: "session_catalog",
            reason: "本机公钥文件超过 64 KiB 上限".to_owned(),
        });
    }
    let trimmed = key_text.trim();
    if !trimmed.starts_with("ssh-") && !trimmed.starts_with("ecdsa-") {
        return Err(ToolError::InvalidArguments {
            tool: "session_catalog",
            reason: "文件不是有效的 OpenSSH 公钥（严禁传入私钥）".to_owned(),
        });
    }
    let escaped = trimmed.replace('\'', "'\"'\"'");
    let cmd = format!(
        "mkdir -p ~/.ssh && chmod 700 ~/.ssh && touch ~/.ssh/authorized_keys && \
         chmod 600 ~/.ssh/authorized_keys && \
         (grep -qxF '{escaped}' ~/.ssh/authorized_keys || echo '{escaped}' >> ~/.ssh/authorized_keys)"
    );
    let out = remote
        .exec_command_capture(&cmd)
        .await
        .map_err(|e| ToolError::ExecutionFailed {
            tool: "session_catalog",
            reason: e.to_string(),
        })?;
    if out.exit_status == Some(0) {
        Ok("已将公钥安全追加到远端 ~/.ssh/authorized_keys".to_owned())
    } else {
        Err(ToolError::ExecutionFailed {
            tool: "session_catalog",
            reason: out.stderr,
        })
    }
}

fn req_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, ToolError> {
    args[key]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ToolError::InvalidArguments {
            tool: "session_catalog",
            reason: format!("缺少必要参数 {key}"),
        })
}

fn parse_session_id(raw: &str) -> Option<SessionId> {
    uuid::Uuid::parse_str(raw.trim()).ok().map(SessionId::from)
}

fn parse_folder_id(raw: &str) -> Option<FolderId> {
    uuid::Uuid::parse_str(raw.trim()).ok().map(FolderId::from)
}

fn parse_protection(raw: &str) -> HostProtection {
    match raw {
        "production" => HostProtection::Production,
        "read_only" => HostProtection::ReadOnly,
        _ => HostProtection::Normal,
    }
}
