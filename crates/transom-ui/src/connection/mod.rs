//! 会话连接建立、自动重连调度与派发流程主模块。

mod auth_dialog;
mod local_and_quick;
mod secrets;

pub(crate) use secrets::describe_target;

use crate::app::{AppShell, TabStatus, TerminalTab};
use crate::palette::localized_error;
use gpui_kit::*;
use secrets::{ConnectPayload, collect_connection_secrets, collect_jump_hops};
use transom_core::{
    AppError, ConnectParams, Connection, ConnectionId, ErrorCode, Session, SessionId, TabId,
    TerminalCommand, TerminalSize,
};

impl AppShell {
    /// 为某个会话新建一个标签组并发起连接。
    pub(crate) fn connect_session(
        &mut self,
        session_id: SessionId,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(found) = self
            .catalog
            .sessions
            .iter()
            .find(|session| session.id == session_id)
        else {
            return;
        };
        let session = self.catalog.resolved_session(found);

        let tab_id = TabId::new();
        self.create_group(tab_id, session_id);
        self.workspace
            .tabs
            .push(TerminalTab::pending(tab_id, session.clone()));
        self.workspace.active_tab = Some(tab_id);
        self.present_active_group();
        self.start_connection(tab_id, session, cx);
    }

    /// 意外断线或连接失败后的自动重连调度。
    pub(crate) fn schedule_auto_reconnect(
        &mut self,
        tab_id: TabId,
        failed_connection: ConnectionId,
        cx: &mut Context<Self>,
    ) {
        const BACKOFF_SECS: [u64; 3] = [2, 5, 10];
        let Some(tab) = self.workspace.tabs.iter_mut().find(|tab| tab.id == tab_id) else {
            return;
        };
        if tab.connection_id != Some(failed_connection) {
            return;
        }
        let auto_reconnect = match &tab.session.connection {
            Connection::Ssh(ssh) => ssh.options.auto_reconnect,
            Connection::Telnet(telnet) => telnet.options.auto_reconnect,
            _ => false,
        };
        if !auto_reconnect {
            return;
        }
        let attempt = tab.reconnect_attempts as usize;
        let Some(delay_secs) = BACKOFF_SECS.get(attempt).copied() else {
            return;
        };
        tab.reconnect_attempts += 1;
        let (code, reason) = match &tab.status {
            TabStatus::Failed(error) => (error.code, error.to_string()),
            _ => (ErrorCode::ConnectionFailed, "连接已断开".to_string()),
        };
        tab.status = TabStatus::Failed(AppError::with_detail(
            code,
            format!(
                "{reason}，{delay_secs} 秒后自动重连（第 {}/3 次）",
                attempt + 1
            ),
        ));
        log::warn!(
            "已安排自动重连，窗格: {tab_id}，会话: {}，延迟 {delay_secs} 秒（第 {}/3 次），原因: {reason}",
            tab.session.name,
            attempt + 1
        );
        let session = tab.session.clone();
        let session_id = tab.session_id;
        let executor = cx.background_executor().clone();
        tab.reconnect_task = Some(cx.spawn(async move |this, cx| {
            executor
                .timer(std::time::Duration::from_secs(delay_secs))
                .await;
            let _ = this.update(cx, |this, cx| {
                let Some(tab) = this.workspace.tabs.iter().find(|tab| tab.id == tab_id) else {
                    return;
                };
                if tab.connection_id != Some(failed_connection) || tab.session_id != session_id {
                    return;
                }
                this.start_connection(tab_id, session, cx);
            });
        }));
        cx.notify();
    }

    pub(crate) fn start_connection(
        &mut self,
        tab_id: TabId,
        session: Session,
        cx: &mut Context<Self>,
    ) {
        let connection_id = ConnectionId::new();
        log::info!(
            "开始建立连接，窗格: {tab_id}，连接: {connection_id:?}，会话: {}，目标: {}",
            session.name,
            describe_target(&session)
        );
        if let Some(tab) = self.workspace.tabs.iter_mut().find(|tab| tab.id == tab_id) {
            tab.connection_id = Some(connection_id);
            tab.sftp = None;
            tab.forward_statuses.clear();
            tab.network_traffic = None;
            tab._startup_task = None;
        }
        if !matches!(
            session.connection,
            Connection::Ssh(_)
                | Connection::Local(_)
                | Connection::Serial(_)
                | Connection::Telnet(_)
        ) {
            self.error = Some(rust_i18n::t!("error.configuration").to_string());
            return;
        }

        let chain = match self.catalog.proxy_jump_chain(&session) {
            Ok(chain) => chain,
            Err(error) => {
                log::warn!(
                    "跳板机链路解析失败，窗格: {tab_id}，会话: {}，原因: {error}",
                    session.name
                );
                self.error = Some(localized_error(&error));
                if let Some(tab) = self.workspace.tabs.iter_mut().find(|tab| tab.id == tab_id) {
                    tab.status = TabStatus::Failed(error);
                }
                cx.notify();
                return;
            }
        };

        let proxy = session.effective_proxy(&self.settings.proxy);
        if let Err(reason) = proxy.validate() {
            log::warn!(
                "出站代理配置非法，窗格: {tab_id}，会话: {}，原因: {reason}",
                session.name
            );
            let error = AppError::with_detail(ErrorCode::Configuration, reason);
            self.error = Some(localized_error(&error));
            if let Some(tab) = self.workspace.tabs.iter_mut().find(|tab| tab.id == tab_id) {
                tab.status = TabStatus::Failed(error);
            }
            cx.notify();
            return;
        }

        let secret_store = self.services.secret_store.clone();
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
                    this.error = Some(error);
                }
                this.send_connect(
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
        &mut self,
        tab_id: TabId,
        connection_id: ConnectionId,
        session: Session,
        payload: ConnectPayload,
        cx: &mut Context<Self>,
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
            zmodem: self.settings.zmodem.clone(),
            allow_sha1_publickey_auth: self.settings.allow_sha1_publickey_auth,
        }));
        if !self.send_command(command) {
            self.remove_tab(tab_id);
        }
        cx.notify();
    }
}
