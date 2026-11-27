//! 领域 3 同步、领域 9（快捷指令/设置/云同步/加密备份）。

use std::path::PathBuf;

use serde_json::{Value, json};
use transom_ai::ToolError;
use transom_core::{BackupExportOptions, Snippet, SnippetId};

use super::ControllerAppActionBridge;

pub(super) async fn handle_sftp_transfer(
    bridge: &ControllerAppActionBridge,
    action: &str,
    args: &Value,
) -> Result<String, ToolError> {
    match action {
        "list_transfers" | "cancel_transfer" => {
            bridge
                .request_ui_action("sftp_transfer", action, args)
                .await
        }
        "sync_preview" | "sync_apply" => {
            let Some(sftp) = &bridge.deps.active_sftp else {
                return Err(ToolError::NoActiveSession);
            };
            let local_dir = args["local_dir"]
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| ToolError::InvalidArguments {
                    tool: "sftp_transfer",
                    reason: "必须提供 local_dir".to_owned(),
                })?;
            let remote_dir = args["remote_dir"]
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| ToolError::InvalidArguments {
                    tool: "sftp_transfer",
                    reason: "必须提供 remote_dir".to_owned(),
                })?;
            let local = std::path::Path::new(local_dir);
            let diff = sftp
                .compare_directories(local, remote_dir, &transom_core::TransferFilter::default())
                .await
                .map_err(|e| ToolError::ExecutionFailed {
                    tool: "sftp_transfer",
                    reason: e.to_string(),
                })?;
            if action == "sync_preview" {
                let summary: Vec<Value> = diff
                    .entries
                    .iter()
                    .take(100)
                    .map(|e| {
                        json!({
                            "path": e.relative_path,
                            "status": format!("{:?}", e.status),
                        })
                    })
                    .collect();
                return serde_json::to_string_pretty(&json!({
                    "total_entries": diff.entries.len(),
                    "entries": summary,
                }))
                .map_err(|e| ToolError::ExecutionFailed {
                    tool: "sftp_transfer",
                    reason: e.to_string(),
                });
            }
            let dir_filter = args["direction"].as_str().unwrap_or("both");
            let items: Vec<transom_core::SyncItem> = diff
                .entries
                .iter()
                .filter_map(|e| e.sync_item())
                .filter(|item| match dir_filter {
                    "upload" => item.direction == transom_core::SyncDirection::Upload,
                    "download" => item.direction == transom_core::SyncDirection::Download,
                    _ => true,
                })
                .collect();
            let report = sftp
                .sync_items(
                    local,
                    remote_dir,
                    &items,
                    transom_core::VerifyOptions::default(),
                    |_, _| {},
                )
                .await
                .map_err(|e| ToolError::ExecutionFailed {
                    tool: "sftp_transfer",
                    reason: e.to_string(),
                })?;
            Ok(format!("目录同步完成（共 {} 项）：{report:?}", items.len()))
        }
        other => Err(ToolError::InvalidArguments {
            tool: "sftp_transfer",
            reason: format!("未知 SFTP 同步动作: {other}"),
        }),
    }
}

pub(super) async fn handle_settings(
    bridge: &ControllerAppActionBridge,
    action: &str,
    args: &Value,
) -> Result<String, ToolError> {
    match action {
        "list_snippets" => list_snippets(bridge).await,
        "save_snippet" => save_snippet(bridge, args).await,
        "delete_snippet" => delete_snippet(bridge, args).await,
        "get_settings" => get_settings(bridge).await,
        "update_settings" => update_settings(bridge, args).await,
        "sync_status" | "sync_push" | "sync_pull" => handle_sync(bridge, action).await,
        "export_backup" => export_backup(bridge, args).await,
        other => Err(ToolError::InvalidArguments {
            tool: "app_settings_ops",
            reason: format!("未知设置动作: {other}"),
        }),
    }
}

async fn list_snippets(bridge: &ControllerAppActionBridge) -> Result<String, ToolError> {
    let store = bridge.deps.settings_store.clone();
    let settings = tokio::task::spawn_blocking(move || store.load())
        .await
        .map_err(|e| ToolError::ExecutionFailed {
            tool: "app_settings_ops",
            reason: e.to_string(),
        })?
        .map_err(|e| ToolError::ExecutionFailed {
            tool: "app_settings_ops",
            reason: e.to_string(),
        })?;
    serde_json::to_string_pretty(&settings.snippets.items).map_err(|e| ToolError::ExecutionFailed {
        tool: "app_settings_ops",
        reason: e.to_string(),
    })
}

async fn save_snippet(
    bridge: &ControllerAppActionBridge,
    args: &Value,
) -> Result<String, ToolError> {
    let name = args["name"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ToolError::InvalidArguments {
            tool: "app_settings_ops",
            reason: "save_snippet 必须提供 name".to_owned(),
        })?
        .to_owned();
    let command = args["command"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ToolError::InvalidArguments {
            tool: "app_settings_ops",
            reason: "save_snippet 必须提供 command".to_owned(),
        })?
        .to_owned();
    let snippet = Snippet {
        id: args["snippet_id"]
            .as_str()
            .and_then(|s| uuid::Uuid::parse_str(s).ok())
            .map(SnippetId::from)
            .unwrap_or_default(),
        name: name.clone(),
        command,
    };
    if !snippet.is_valid() {
        return Err(ToolError::InvalidArguments {
            tool: "app_settings_ops",
            reason: "快捷指令名称（<=40字）或命令长度超出限制".to_owned(),
        });
    }
    mutate_settings_and_reload(bridge, move |settings| {
        if let Some(pos) = settings
            .snippets
            .items
            .iter()
            .position(|s| s.id == snippet.id)
        {
            settings.snippets.items[pos] = snippet;
        } else {
            settings.snippets.items.push(snippet);
        }
        settings.snippets = std::mem::take(&mut settings.snippets).sanitized();
        Ok(())
    })
    .await?;
    Ok(format!("已保存快捷指令「{name}」"))
}

async fn mutate_settings_and_reload(
    bridge: &ControllerAppActionBridge,
    mutator: impl FnOnce(&mut transom_core::AppSettings) -> Result<(), transom_core::AppError>
    + Send
    + 'static,
) -> Result<(), ToolError> {
    let store = bridge.deps.settings_store.clone();
    let settings = tokio::task::spawn_blocking(move || {
        let mut settings = store.load()?;
        mutator(&mut settings)?;
        store.save(&settings)?;
        Ok::<_, transom_core::AppError>(settings)
    })
    .await
    .map_err(|e| ToolError::ExecutionFailed {
        tool: "app_settings_ops",
        reason: e.to_string(),
    })?
    .map_err(|e| ToolError::ExecutionFailed {
        tool: "app_settings_ops",
        reason: e.to_string(),
    })?;

    let payload = json!({ "settings": settings });
    let _ = bridge
        .request_ui_action("app_settings_ops", "reload_settings", &payload)
        .await;
    Ok(())
}

async fn delete_snippet(
    bridge: &ControllerAppActionBridge,
    args: &Value,
) -> Result<String, ToolError> {
    let target = args["snippet_id"]
        .as_str()
        .or_else(|| args["name"].as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ToolError::InvalidArguments {
            tool: "app_settings_ops",
            reason: "delete_snippet 必须提供 snippet_id 或 name".to_owned(),
        })?
        .to_owned();
    let target_out = target.clone();
    mutate_settings_and_reload(bridge, move |settings| {
        let before = settings.snippets.items.len();
        settings
            .snippets
            .items
            .retain(|s| s.id.to_string() != target && s.name != target);
        if settings.snippets.items.len() == before {
            return Err(transom_core::AppError::with_detail(
                transom_core::ErrorCode::Configuration,
                format!("未找到快捷指令: {target}"),
            ));
        }
        Ok(())
    })
    .await?;
    Ok(format!("已删除快捷指令: {target_out}"))
}

async fn get_settings(bridge: &ControllerAppActionBridge) -> Result<String, ToolError> {
    let store = bridge.deps.settings_store.clone();
    let s = tokio::task::spawn_blocking(move || store.load())
        .await
        .map_err(|e| ToolError::ExecutionFailed {
            tool: "app_settings_ops",
            reason: e.to_string(),
        })?
        .map_err(|e| ToolError::ExecutionFailed {
            tool: "app_settings_ops",
            reason: e.to_string(),
        })?;
    // 脱敏摘要：不输出任何同步密钥或代理凭据。
    serde_json::to_string_pretty(&json!({
        "language": s.language,
        "theme_mode": format!("{:?}", s.appearance.mode),
        "ui_theme": s.appearance.ui_theme,
        "snippet_count": s.snippets.items.len(),
        "sync_enabled": s.sync.enabled,
        "audit_enabled": s.audit.enabled,
    }))
    .map_err(|e| ToolError::ExecutionFailed {
        tool: "app_settings_ops",
        reason: e.to_string(),
    })
}

async fn update_settings(
    bridge: &ControllerAppActionBridge,
    args: &Value,
) -> Result<String, ToolError> {
    let lang = args["language"].as_str().map(str::to_owned);
    let theme = args["theme"].as_str().map(str::to_owned);
    mutate_settings_and_reload(bridge, move |s| {
        if let Some(l) = lang {
            s.language = l;
        }
        if let Some(t) = theme {
            s.appearance.ui_theme = t;
        }
        Ok(())
    })
    .await?;
    Ok("已更新应用设置并通知界面刷新".to_owned())
}

async fn handle_sync(
    bridge: &ControllerAppActionBridge,
    action: &str,
) -> Result<String, ToolError> {
    let settings_store = bridge.deps.settings_store.clone();
    let settings = tokio::task::spawn_blocking(move || settings_store.load())
        .await
        .map_err(|e| ToolError::ExecutionFailed {
            tool: "app_settings_ops",
            reason: e.to_string(),
        })?
        .map_err(|e| ToolError::ExecutionFailed {
            tool: "app_settings_ops",
            reason: e.to_string(),
        })?;

    if action == "sync_status" {
        return serde_json::to_string_pretty(&json!({
            "enabled": settings.sync.enabled,
            "channel": format!("{:?}", settings.sync.active_channel),
        }))
        .map_err(|e| ToolError::ExecutionFailed {
            tool: "app_settings_ops",
            reason: e.to_string(),
        });
    }

    bridge
        .request_ui_action(
            "app_settings_ops",
            action,
            &json!({ "sync_settings": settings.sync }),
        )
        .await
}

async fn export_backup(
    bridge: &ControllerAppActionBridge,
    args: &Value,
) -> Result<String, ToolError> {
    let output_path = args["output_path"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ToolError::InvalidArguments {
            tool: "app_settings_ops",
            reason: "export_backup 必须提供 output_path".to_owned(),
        })?;
    let passphrase = args["passphrase"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ToolError::InvalidArguments {
            tool: "app_settings_ops",
            reason: "出于安全原因，导出备份必须提供加密口令 passphrase".to_owned(),
        })?
        .to_owned();
    let path = PathBuf::from(output_path);
    let session_store = bridge.deps.session_store.clone();
    let backup = bridge.deps.backup.clone();
    let report = tokio::task::spawn_blocking(move || {
        let catalog = session_store.load()?;
        let opts = BackupExportOptions {
            passphrase: Some(zeroize::Zeroizing::new(passphrase)),
            include_secrets: false,
        };
        backup.export(&path, &catalog, &opts)
    })
    .await
    .map_err(|e| ToolError::ExecutionFailed {
        tool: "app_settings_ops",
        reason: e.to_string(),
    })?
    .map_err(|e| ToolError::ExecutionFailed {
        tool: "app_settings_ops",
        reason: e.to_string(),
    })?;
    Ok(format!(
        "已导出加密备份到 {output_path}（包含 {} 条会话、{} 个分组）",
        report.sessions, report.folders
    ))
}
