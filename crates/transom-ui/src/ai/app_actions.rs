//! UI 侧软件级动作执行器：响应控制平面发来的 `AppActionRequested`。
//! 返回值面向 AI 模型/MCP 调用方，采用结构化英文或 JSON 以符合 UI 源码 i18n 守卫。

use std::path::PathBuf;

use gpui_kit::App;
use serde_json::{Value, json};
use transom_core::{
    BindAddress, CloudPullMode, CloudPushMode, ForwardCommand, ForwardKind, ForwardRule,
    HostCommand, SftpCommand, SocksOptions, SplitAxis, SyncCommand, SyncTrigger, TabId,
    TargetAddress, TransferId,
};

use crate::app::{AppState, update_app_settings};
use crate::sidebar::{SessionSidebarEvent, SidebarSection};

/// 在 GPUI 主线程执行来自 AI 助手的软件操作，返回面向模型的可读结果。
pub(crate) fn handle_app_action(
    state: &mut AppState,
    domain: &str,
    action: &str,
    args_json: &str,
    cx: &mut App,
) -> Result<String, String> {
    let args: Value = serde_json::from_str(args_json).unwrap_or(Value::Null);
    match domain {
        "session_catalog" => handle_catalog(state, action, &args),
        "app_settings_ops" => handle_settings(state, action, &args, cx),
        "workspace_control" => handle_workspace(state, action, &args, cx),
        "host_monitor" => handle_monitor(state, action, &args, cx),
        "port_forward_ops" => handle_forwards(state, action, &args),
        "sftp_transfer" => handle_sftp(state, action, &args, cx),
        "container_workspace" => handle_container(state, action),
        other => Err(format!("Unsupported UI domain: {other}")),
    }
}

fn handle_catalog(state: &mut AppState, action: &str, args: &Value) -> Result<String, String> {
    if action != "reload_catalog" {
        return Err(format!("Unknown session_catalog UI action: {action}"));
    }
    let catalog_val = args
        .get("catalog")
        .ok_or_else(|| "reload_catalog missing catalog payload".to_owned())?;
    let catalog: transom_core::SessionCatalog =
        serde_json::from_value(catalog_val.clone()).map_err(|e| e.to_string())?;
    state.session.catalog = catalog;
    state.session.catalog_revision = state.session.catalog_revision.saturating_add(1);
    state.session.sidebar.rows_cache = None;
    Ok("Catalog reloaded".to_owned())
}

fn handle_settings(
    state: &mut AppState,
    action: &str,
    args: &Value,
    cx: &mut App,
) -> Result<String, String> {
    match action {
        "reload_settings" => {
            let settings_val = args
                .get("settings")
                .ok_or_else(|| "reload_settings missing settings payload".to_owned())?;
            let loaded: transom_core::AppSettings =
                serde_json::from_value(settings_val.clone()).map_err(|e| e.to_string())?;
            update_app_settings(cx, |s| *s = loaded);
            Ok("Settings reloaded".to_owned())
        }
        "sync_push" => {
            let sync_val = args
                .get("sync_settings")
                .ok_or_else(|| "sync_push missing sync_settings payload".to_owned())?;
            let sync_settings: transom_core::SyncSettings =
                serde_json::from_value(sync_val.clone()).map_err(|e| e.to_string())?;
            state.send_command(SyncCommand::CloudSyncPush {
                settings: sync_settings,
                catalog: state.session.catalog.clone(),
                trigger: SyncTrigger::Manual,
                mode: CloudPushMode::Merge,
            });
            Ok("Cloud sync push queued".to_owned())
        }
        "sync_pull" => {
            let sync_val = args
                .get("sync_settings")
                .ok_or_else(|| "sync_pull missing sync_settings payload".to_owned())?;
            let sync_settings: transom_core::SyncSettings =
                serde_json::from_value(sync_val.clone()).map_err(|e| e.to_string())?;
            state.send_command(SyncCommand::CloudSyncPull {
                settings: sync_settings,
                trigger: SyncTrigger::Manual,
                mode: CloudPullMode::Merge,
            });
            Ok("Cloud sync pull queued".to_owned())
        }
        other => Err(format!("Unknown app_settings_ops UI action: {other}")),
    }
}

fn handle_workspace(
    state: &mut AppState,
    action: &str,
    args: &Value,
    cx: &mut App,
) -> Result<String, String> {
    match action {
        "list_tabs" => {
            let active = state.workspace.active_tab;
            let list: Vec<Value> = state
                .workspace
                .tabs
                .iter()
                .map(|t| {
                    json!({
                        "tab_id": t.id.to_string(),
                        "title": t.display_name(),
                        "active": Some(t.id) == active,
                        "session_id": t.session_id().map(|id| id.to_string()),
                    })
                })
                .collect();
            serde_json::to_string_pretty(&list).map_err(|e| e.to_string())
        }
        "connect_session" => {
            let sid = args["session_id"].as_str().unwrap_or("").trim();
            let name = args["name"].as_str().unwrap_or("").trim();
            let found = state
                .session
                .catalog
                .sessions
                .iter()
                .find(|s| {
                    (!sid.is_empty() && s.id.to_string() == sid)
                        || (!name.is_empty() && s.name == name)
                })
                .ok_or_else(|| "Matching saved session not found".to_owned())?;
            let label = found.name.clone();
            state
                .ui
                .window_requests
                .push_session_sidebar(SessionSidebarEvent::ConnectSession(found.id));
            Ok(format!("Connecting session: {label}"))
        }
        "open_local_terminal" => {
            let cwd = args["cwd"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(PathBuf::from);
            state.ui.window_requests.push_local_terminal(cwd);
            Ok("Opened new local terminal tab".to_owned())
        }
        "switch_tab" => {
            let tid = parse_tab_id(args)?;
            state.ui.window_requests.push_activate_tab(tid);
            Ok(format!("Switched to tab: {tid}"))
        }
        "close_tab" => {
            let tid = parse_tab_id(args)?;
            state.ui.window_requests.push_close_tab(tid);
            Ok(format!("Closed tab: {tid}"))
        }
        "split_pane" => {
            let axis = match args["axis"].as_str().unwrap_or("horizontal") {
                "vertical" => SplitAxis::Vertical,
                _ => SplitAxis::Horizontal,
            };
            state.ui.window_requests.push_split_active_pane(axis);
            Ok("Split pane requested".to_owned())
        }
        "open_panel" => open_panel(state, args, cx),
        "send_terminal_input" => {
            let tid = args["tab_id"]
                .as_str()
                .and_then(|s| uuid::Uuid::parse_str(s).ok())
                .map(TabId::from)
                .or(state.workspace.active_tab)
                .ok_or_else(|| "No active terminal tab".to_owned())?;
            let text = args["input"]
                .as_str()
                .ok_or_else(|| "send_terminal_input requires input".to_owned())?;
            let view = state
                .workspace
                .tab(tid)
                .and_then(|t| t.terminal())
                .ok_or_else(|| "Target tab has no terminal view".to_owned())?;
            let ok = view.update(cx, |v, cx| v.send_text(text, cx));
            if ok {
                Ok("Sent input to terminal".to_owned())
            } else {
                Err("Terminal not ready for input".to_owned())
            }
        }
        other => Err(format!("Unknown workspace_control UI action: {other}")),
    }
}

fn open_panel(state: &mut AppState, args: &Value, cx: &mut App) -> Result<String, String> {
    let panel = args["panel"].as_str().unwrap_or("sessions");
    match panel {
        "sessions" => state.session.sidebar.show_section(SidebarSection::Sessions),
        "containers" => state
            .session
            .sidebar
            .show_section(SidebarSection::Containers),
        "git" => state.session.sidebar.show_section(SidebarSection::Git),
        "api" => state
            .session
            .sidebar
            .show_section(SidebarSection::ApiClient),
        "sync" => state
            .session
            .sidebar
            .show_section(SidebarSection::CloudSync),
        "ai" => state.features.ai_panel_open = true,
        "monitor" => state.features.monitor.update(cx, |m, _| m.set_open(true)),
        "sftp" => state.ui.window_requests.push_open_sftp_panel(),
        "forwards" => state.ui.window_requests.push_open_forwards_panel(),
        _ => return Err(format!("Unknown panel: {panel}")),
    }
    Ok(format!("Opened panel: {panel}"))
}

fn handle_monitor(
    state: &mut AppState,
    action: &str,
    args: &Value,
    _cx: &mut App,
) -> Result<String, String> {
    if action != "toggle_sampling" {
        return Err(format!("Unknown host_monitor UI action: {action}"));
    }
    let enabled = args["enabled"].as_bool().unwrap_or(true);
    state.features.stats_sampling = enabled;
    state.send_command(HostCommand::SetStatsSampling { enabled });
    Ok(format!("Host stats sampling enabled={enabled}"))
}

fn handle_forwards(state: &mut AppState, action: &str, args: &Value) -> Result<String, String> {
    let tid = args["tab_id"]
        .as_str()
        .and_then(|s| uuid::Uuid::parse_str(s).ok())
        .map(TabId::from)
        .or(state.workspace.active_tab)
        .ok_or_else(|| "No active SSH tab".to_owned())?;
    let tab = state
        .workspace
        .tab(tid)
        .ok_or_else(|| "Tab not found".to_owned())?;
    let cid = tab
        .connection_id()
        .ok_or_else(|| "Tab is not connected".to_owned())?;
    let sid = tab
        .session_id()
        .ok_or_else(|| "Tab has no saved session".to_owned())?;

    match action {
        "list" => {
            let rules = state
                .session
                .catalog
                .sessions
                .iter()
                .find(|s| s.id == sid)
                .map(|s| s.forwards.clone())
                .unwrap_or_default();
            serde_json::to_string_pretty(&rules).map_err(|e| e.to_string())
        }
        "create" => {
            let listen_host = args["listen_host"]
                .as_str()
                .unwrap_or("127.0.0.1")
                .to_owned();
            let listen_port = args["listen_port"]
                .as_u64()
                .ok_or_else(|| "create requires listen_port".to_owned())?
                as u16;
            let target_host = args["target_host"]
                .as_str()
                .unwrap_or("127.0.0.1")
                .to_owned();
            let target_port = args["target_port"].as_u64().unwrap_or(0) as u16;
            let bind = BindAddress {
                host: listen_host,
                port: listen_port,
            };
            let target = TargetAddress {
                host: target_host,
                port: target_port,
            };
            let kind = match args["kind"].as_str().unwrap_or("local") {
                "remote" => ForwardKind::Remote { bind, target },
                "dynamic" => ForwardKind::Dynamic {
                    bind,
                    socks: SocksOptions::default(),
                },
                _ => ForwardKind::Local { bind, target },
            };
            let rule = ForwardRule::new(args["name"].as_str().unwrap_or("AI Forward"), kind);
            let rule_id = rule.id;
            if let Some(session) = state
                .session
                .catalog
                .sessions
                .iter_mut()
                .find(|s| s.id == sid)
            {
                session.forwards.push(rule.clone());
                session.touch();
                if !state.persist_catalog() {
                    return Err("Failed to save session catalog".to_owned());
                }
            }
            state.send_command(ForwardCommand::ForwardStart {
                tab_id: tid,
                connection_id: cid,
                rule,
            });
            Ok(format!("Created and started forward_id: {rule_id}"))
        }
        "start" | "stop" | "probe" => {
            let fid = parse_forward_id(args)?;
            let rule = state
                .session
                .catalog
                .sessions
                .iter()
                .find(|s| s.id == sid)
                .and_then(|s| s.forwards.iter().find(|r| r.id == fid))
                .cloned()
                .ok_or_else(|| format!("Forward rule not found: {fid}"))?;
            match action {
                "start" => {
                    state.send_command(ForwardCommand::ForwardStart {
                        tab_id: tid,
                        connection_id: cid,
                        rule,
                    });
                }
                "stop" => {
                    state.send_command(ForwardCommand::ForwardStop {
                        tab_id: tid,
                        connection_id: cid,
                        rule_id: fid,
                    });
                }
                _ => {
                    state.send_command(ForwardCommand::ForwardProbe {
                        tab_id: tid,
                        connection_id: cid,
                        rule_id: fid,
                        rule,
                    });
                }
            }
            Ok(format!("Dispatched forward {action}: {fid}"))
        }
        "delete" => {
            let fid = parse_forward_id(args)?;
            state.send_command(ForwardCommand::ForwardStop {
                tab_id: tid,
                connection_id: cid,
                rule_id: fid,
            });
            if let Some(session) = state
                .session
                .catalog
                .sessions
                .iter_mut()
                .find(|s| s.id == sid)
            {
                session.forwards.retain(|r| r.id != fid);
                session.touch();
                if !state.persist_catalog() {
                    return Err("Failed to save session catalog".to_owned());
                }
            }
            Ok(format!("Deleted forward rule: {fid}"))
        }
        other => Err(format!("Unknown port_forward_ops action: {other}")),
    }
}

fn handle_sftp(
    state: &mut AppState,
    action: &str,
    args: &Value,
    _cx: &mut App,
) -> Result<String, String> {
    let tid = args["tab_id"]
        .as_str()
        .and_then(|s| uuid::Uuid::parse_str(s).ok())
        .map(TabId::from)
        .or(state.workspace.active_tab)
        .ok_or_else(|| "No active SFTP tab".to_owned())?;
    match action {
        "list_transfers" => Ok(format!("SFTP queue ready for tab {tid}")),
        "cancel_transfer" => {
            let op_id = args["transfer_id"]
                .as_str()
                .and_then(|s| uuid::Uuid::parse_str(s).ok())
                .map(TransferId::from)
                .ok_or_else(|| "cancel_transfer requires valid transfer_id".to_owned())?;
            state.send_command(SftpCommand::SftpCancel { tab_id: tid, op_id });
            Ok(format!("Requested transfer cancel: {op_id}"))
        }
        other => Err(format!("Unknown sftp_transfer UI action: {other}")),
    }
}

fn handle_container(state: &mut AppState, action: &str) -> Result<String, String> {
    if action == "open_exec_terminal" {
        state
            .session
            .sidebar
            .show_section(SidebarSection::Containers);
        return Ok("Switched to container sidebar".to_owned());
    }
    Err(format!("Unknown container_workspace UI action: {action}"))
}

fn parse_tab_id(args: &Value) -> Result<TabId, String> {
    args["tab_id"]
        .as_str()
        .and_then(|s| uuid::Uuid::parse_str(s).ok())
        .map(TabId::from)
        .ok_or_else(|| "Missing valid tab_id".to_owned())
}

fn parse_forward_id(args: &Value) -> Result<transom_core::ForwardId, String> {
    args["forward_id"]
        .as_str()
        .and_then(|s| uuid::Uuid::parse_str(s).ok())
        .map(transom_core::ForwardId::from)
        .ok_or_else(|| "Missing valid forward_id".to_owned())
}
