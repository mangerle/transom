//! 会话连接建立、自动重连调度与派发流程主模块。

mod auth_dialog;
mod local_and_quick;
mod local_shell_menu;
mod presentation;
mod prompts_render;
mod prompts_shell;
mod prompts_view;
mod rdp;
mod secrets;

pub(crate) use auth_dialog::flush_pending_credential;
pub(crate) use local_and_quick::{
    connect_quick_host, handle_deep_link, import_external_sessions, import_ssh_config,
    on_new_local_terminal, on_quick_open, open_local_terminal, record_history_command,
    spawn_local_command, spawn_local_shell, spawn_local_terminal, toggle_quick_open,
};
pub(crate) use local_shell_menu::build_local_terminal_menu;
pub(crate) use presentation::{describe_target, display_compact, display_with_port, icon};
pub(crate) use prompts_shell::{on_connection_prompts_event, request_clipboard_permission};
pub(crate) use prompts_view::{
    ClipboardWritePrompt, ConnectionPromptsEvent, ConnectionPromptsView,
};
use rdp::open_rdp_session;
pub(crate) use secrets::{collect_connection_secrets, session_host};

use crate::app::app_settings;
use crate::app::app_settings_mut;
use crate::app::{AppShell, AppState, TabStatus, WorkspaceTab};
use crate::palette::localized_error;
use gpui_kit::component::input::InputState;
use gpui_kit::*;
use secrets::{ConnectPayload, collect_jump_hops};
use transom_core::{
    AppError, ConnectParams, Connection, ConnectionId, ErrorCode, PromptInfo, SecretKind,
    SecretOwner, Session, SessionId, TabId, TerminalCommand, TerminalSize,
};
use zeroize::Zeroizing;

/// 认证通过前暂存的待保存凭据。
///
/// 只有连接真正建立后才允许写入钥匙串：口令被拒绝或连接取消时，
/// 这条意图随连接一起作废，避免把未经验证的口令持久化。
pub(crate) struct PendingCredential {
    pub(crate) session_id: SessionId,
    pub(crate) owner: SecretOwner,
    pub(crate) connection_id: ConnectionId,
    pub(crate) kind: SecretKind,
    pub(crate) value: Zeroizing<String>,
}

/// 键盘交互式认证弹窗的状态与回答通道。
pub(crate) struct AuthPromptState {
    pub(crate) connection_id: ConnectionId,
    pub(crate) name: String,
    pub(crate) instructions: String,
    pub(crate) prompts: Vec<PromptInfo>,
    pub(crate) inputs: Vec<Entity<InputState>>,
    pub(crate) responder: tokio::sync::mpsc::Sender<Vec<String>>,
    pub(crate) error: Option<String>,
    pub(crate) session_id: SessionId,
    pub(crate) secret_kind: Option<SecretKind>,
    pub(crate) save: bool,
}

/// 首次连接的主机指纹确认弹窗状态。
pub(crate) struct HostKeyConfirmState {
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) key_type: String,
    pub(crate) fingerprint: String,
    pub(crate) responder: tokio::sync::mpsc::Sender<bool>,
}

/// 记录最近打开的会话 ID，并异步落盘保存。
/// 从最近打开会话记录中移除指定会话，并异步落盘保存。
pub(crate) fn remove_recent_session(
    app_state: &mut AppState,
    session_id: SessionId,
    cx: &mut Context<AppShell>,
) {
    let before_len = app_settings(cx).window.recent_sessions.len();
    app_settings_mut(cx)
        .window
        .recent_sessions
        .retain(|id| *id != session_id);
    if app_settings(cx).window.recent_sessions.len() != before_len {
        let settings_snapshot = app_settings(cx).clone();
        if let Err(error) = app_state.persist_settings(&settings_snapshot) {
            log::warn!("保存最近打开会话设置失败，原因: {error}");
        }
        cx.notify();
    }
}

/// 为某个会话新建一个标签组并发起连接。
pub(crate) fn connect_session(
    app_state: &mut AppState,
    session_id: SessionId,
    _window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    let Some(found) = app_state
        .session
        .catalog
        .sessions
        .iter()
        .find(|session| session.id == session_id)
    else {
        return;
    };
    let session = app_state.session.catalog.resolved_session(found);
    crate::connection::record_recent_session(app_state, session_id, cx);

    let tab_id = TabId::new();
    if let Connection::Rdp(rdp) = &session.connection {
        open_rdp_session(app_state, tab_id, session.clone(), rdp.clone(), cx);
        return;
    }

    let tab = WorkspaceTab::new_session(tab_id, session.clone());
    app_state.workspace.create_group_for_tab(&tab);
    app_state.workspace.tabs.push(tab);
    app_state.workspace.active_tab = Some(tab_id);
    app_state.present_active_group();
    crate::connection::start_connection(app_state, tab_id, session, cx);
}

/// 判断连接错误码是否属于可自动重连的瞬态网络/通道故障。
///
/// 认证失败、主机指纹校验不通过、配置错误或用户主动取消等确定性失败不应自动重连，
/// 避免触发远端防爆破封锁或无效重复弹窗。
pub(crate) fn is_auto_reconnectable(code: ErrorCode) -> bool {
    matches!(
        code,
        ErrorCode::ConnectionFailed
            | ErrorCode::Dns
            | ErrorCode::Timeout
            | ErrorCode::ChannelFailed
            | ErrorCode::Io
            | ErrorCode::Protocol
    )
}

/// 意外断线或连接失败后的自动重连调度。
pub(crate) fn schedule_auto_reconnect(
    app_state: &mut AppState,
    tab_id: TabId,
    failed_connection: ConnectionId,
    cx: &mut Context<AppShell>,
) {
    const BACKOFF_SECS: [u64; 4] = [2, 5, 10, 20];
    let Some(tab) = app_state.workspace.tab_mut(tab_id) else {
        return;
    };
    if tab.connection_id() != Some(failed_connection) {
        return;
    }
    if let TabStatus::Failed(error) = &tab.status
        && !is_auto_reconnectable(error.code)
    {
        log::info!(
            "跳过自动重连，窗格: {tab_id}，错误类型 {:?} 需人工干预",
            error.code
        );
        return;
    }
    let Some(session_id) = tab.session_id() else {
        return;
    };
    let Some(session) = tab.session().cloned() else {
        return;
    };
    let auto_reconnect = match &session.connection {
        Connection::Ssh(ssh) => ssh.options.auto_reconnect,
        Connection::Telnet(telnet) => telnet.options.auto_reconnect,
        _ => false,
    };
    if !auto_reconnect {
        return;
    }
    let attempt = tab
        .session_pane()
        .map(|p| p.reconnect_attempts as usize)
        .unwrap_or(0);
    let Some(delay_secs) = BACKOFF_SECS.get(attempt).copied() else {
        if let Some(pane) = tab.session_pane_mut() {
            pane.reconnect_detail = None;
            pane.reconnect_task = None;
        }
        return;
    };
    let total_attempts = BACKOFF_SECS.len();
    let (code, reason) = match &tab.status {
        TabStatus::Failed(error) => (error.code, error.to_string()),
        _ => (ErrorCode::ConnectionFailed, "连接已断开".to_string()),
    };
    let detail = format!(
        "{reason}，{delay_secs} 秒后自动重连（第 {}/{total_attempts} 次）",
        attempt + 1
    );
    tab.status = TabStatus::Failed(AppError::with_detail(code, detail.clone()));
    let session_name = session.name.clone();
    log::warn!(
        "已安排自动重连，窗格: {tab_id}，会话: {}，延迟 {delay_secs} 秒（第 {}/{total_attempts} 次），原因: {reason}",
        session_name,
        attempt + 1
    );
    let executor = cx.background_executor().clone();
    let Some(pane) = tab.session_pane_mut() else {
        return;
    };
    pane.reconnect_attempts += 1;
    pane.reconnect_detail = Some(detail);
    pane.reconnect_task = Some(cx.spawn(async move |this, cx| {
        executor
            .timer(std::time::Duration::from_secs(delay_secs))
            .await;
        let _ = this.update(cx, |this, cx| {
            let Some(tab) = this.state.workspace.tab(tab_id) else {
                return;
            };
            if tab.connection_id() != Some(failed_connection)
                || tab.session_id() != Some(session_id)
            {
                return;
            }
            crate::connection::start_connection(&mut this.state, tab_id, session, cx);
        });
    }));
    cx.notify();
}

/// 新连接建立前清空旧通道残留：连接标识、cwd 与面板快照。
fn reset_tab_connection(app_state: &mut AppState, tab_id: TabId, connection_id: ConnectionId) {
    if let Some(tab) = app_state.workspace.tab_mut(tab_id) {
        tab.set_connection_id(Some(connection_id));
        // 新通道上的 Shell 还没上报 OSC 7，旧目录不再代表当前 cwd；
        // 等新连接上的提示符钩子回报后再恢复 SFTP 跟随。
        tab.set_cwd(None);
        // 旧连接的进度条属于已经结束的任务，不能带到新通道上继续显示。
        tab.set_progress(None);
        if let Some(pane) = tab.session_pane_mut() {
            pane.sftp_view = None;
            pane.forward_statuses.clear();
            pane.network_traffic = None;
            pane._startup_task = None;
            pane.reconnect_task = None;
            pane.reconnect_detail = None;
        }
    }
}

pub(crate) fn start_connection(
    app_state: &mut AppState,
    tab_id: TabId,
    session: Session,
    cx: &mut Context<AppShell>,
) {
    let connection_id = ConnectionId::new();
    log::info!(
        "开始建立连接，窗格: {tab_id}，连接: {connection_id:?}，会话: {}，目标: {}",
        session.name,
        describe_target(&session)
    );
    reset_tab_connection(app_state, tab_id, connection_id);

    if !connection_supported(&session) {
        app_state.set_error(rust_i18n::t!("error.configuration").to_string());
        return;
    }

    let chain = match app_state.session.catalog.proxy_jump_chain(&session) {
        Ok(chain) => chain,
        Err(error) => {
            log::warn!(
                "跳板机链路解析失败，窗格: {tab_id}，会话: {}，原因: {error}",
                session.name
            );
            app_state.set_error(localized_error(&error));
            if let Some(tab) = app_state.workspace.tab_mut(tab_id) {
                tab.status = TabStatus::Failed(error);
            }
            cx.notify();
            return;
        }
    };

    let proxy = session.effective_proxy(&app_settings(cx).proxy);
    if let Err(reason) = proxy.validate() {
        log::warn!(
            "出站代理配置非法，窗格: {tab_id}，会话: {}，原因: {reason}",
            session.name
        );
        let error = AppError::with_detail(ErrorCode::Configuration, reason);
        app_state.set_error(localized_error(&error));
        if let Some(tab) = app_state.workspace.tab_mut(tab_id) {
            tab.status = TabStatus::Failed(error);
        }
        cx.notify();
        return;
    }

    let secret_store = app_state.runtime.services.secret_store.clone();
    let load_session = session.clone();
    cx.spawn(async move |this, cx| {
        let (secrets, hops, error) = cx
            .background_executor()
            .spawn(async move {
                let (secrets, mut error) =
                    collect_connection_secrets(secret_store.as_ref(), &load_session);
                let (hops, hop_error) = collect_jump_hops(secret_store.as_ref(), &chain);
                if error.is_none() {
                    error = hop_error;
                }
                (secrets, hops, error)
            })
            .await;
        let _ = this.update(cx, |this, cx| {
            if let Some(error) = error {
                this.state.set_error(error);
            }
            crate::connection::send_connect(
                &mut this.state,
                tab_id,
                connection_id,
                session,
                ConnectPayload {
                    jumps: hops,
                    proxy,
                    secrets,
                },
                cx,
            );
        });
    })
    .detach();
}

pub(crate) fn send_connect(
    app_state: &mut AppState,
    tab_id: TabId,
    connection_id: ConnectionId,
    session: Session,
    payload: ConnectPayload,
    cx: &mut Context<AppShell>,
) {
    let size = TerminalSize::new(120, 32);
    let command = TerminalCommand::Connect(Box::new(ConnectParams {
        tab_id,
        connection_id,
        session,
        jumps: payload.jumps,
        proxy: payload.proxy,
        secrets: payload.secrets,
        size,
        zmodem: app_settings(cx).zmodem.clone(),
        allow_sha1_publickey_auth: app_settings(cx).allow_sha1_publickey_auth,
        shell_integration: app_settings(cx).block_view.shell_integration,
    }));
    if !app_state.send_command(command) {
        app_state.remove_tab(tab_id);
        crate::editor::prune_tab_overlays(app_state, cx);
    }
    cx.notify();
}

pub(crate) fn record_recent_session(
    app_state: &mut AppState,
    session_id: SessionId,
    cx: &mut Context<AppShell>,
) {
    app_settings_mut(cx)
        .window
        .recent_sessions
        .retain(|id| *id != session_id);
    app_settings_mut(cx)
        .window
        .recent_sessions
        .insert(0, session_id);
    app_settings_mut(cx).window.recent_sessions.truncate(20);
    let settings_snapshot = app_settings_mut(cx).clone();
    if let Err(error) = app_state.persist_settings(&settings_snapshot) {
        log::warn!("保存最近打开会话失败，原因: {error}");
    }
}

/// 当前支持通过界面建立的连接类型；未知类型在派发前拒绝。
///
/// FTP 不产生终端，连接就绪后由 `FtpReady` 事件打开文件面板作为正文；
/// RDP 走独立图形视口渲染管线，通过 `open_rdp_session` 创建原生图形视图。
fn connection_supported(session: &Session) -> bool {
    matches!(
        session.connection,
        Connection::Ssh(_)
            | Connection::Local(_)
            | Connection::Serial(_)
            | Connection::Telnet(_)
            | Connection::Ftp(_)
            | Connection::Rdp(_)
    )
}

#[cfg(test)]
mod tests {
    use super::is_auto_reconnectable;
    use transom_core::ErrorCode;

    #[test]
    fn transient_network_errors_are_reconnectable() {
        for code in [
            ErrorCode::ConnectionFailed,
            ErrorCode::Dns,
            ErrorCode::Timeout,
            ErrorCode::ChannelFailed,
            ErrorCode::Io,
            ErrorCode::Protocol,
        ] {
            assert!(is_auto_reconnectable(code), "{code:?} 应当允许自动重连");
        }
    }

    #[test]
    fn deterministic_or_auth_errors_skip_auto_reconnect() {
        for code in [
            ErrorCode::Configuration,
            ErrorCode::AuthFailed,
            ErrorCode::KeyboardInteractiveFailed,
            ErrorCode::HostKeyUnknown,
            ErrorCode::HostKeyMismatch,
            ErrorCode::Cancelled,
            ErrorCode::NotSupported,
            ErrorCode::PermissionDenied,
        ] {
            assert!(!is_auto_reconnectable(code), "{code:?} 不应自动重连");
        }
    }
}
