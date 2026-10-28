//! 端口转发规则管理与启停控制方法。

use super::{
    ForwardKindChoice, ForwardPanelState, PendingForwardConfirm, forward_bind_host,
    is_public_bind_address,
};
use crate::app::{AppShell, TabStatus};
use crate::keymap;
use crate::palette::sftp_text_input;
use gpui_kit::*;
use transom_core::{
    AppError, BindAddress, Connection, ConnectionId, ErrorCode, ForwardCommand, ForwardId,
    ForwardKind, ForwardRule, ForwardState, ForwardStatus, SessionId, SocksOptions, TabId,
    TargetAddress,
};

impl AppShell {
    pub(crate) fn mark_forwards_stopped(&mut self, tab_id: TabId) {
        if let Some(pane) = self
            .workspace
            .tab_mut(tab_id)
            .and_then(|tab| tab.session_pane_mut())
        {
            for status in pane.forward_statuses.values_mut() {
                if !matches!(status.state, ForwardState::Stopped) {
                    status.state = ForwardState::Stopped;
                    status.error = None;
                }
            }
        }
    }

    pub(crate) fn set_forward_status(
        &mut self,
        tab_id: TabId,
        rule_id: ForwardId,
        state: ForwardState,
        error: Option<AppError>,
    ) {
        if let Some(pane) = self
            .workspace
            .tab_mut(tab_id)
            .and_then(|tab| tab.session_pane_mut())
        {
            pane.forward_statuses.insert(
                rule_id,
                ForwardStatus {
                    rule_id,
                    state,
                    error,
                },
            );
        }
    }

    pub(crate) fn auto_start_forwards(
        &mut self,
        tab_id: TabId,
        connection_id: ConnectionId,
        session_id: SessionId,
        cx: &mut Context<Self>,
    ) {
        let rules: Vec<ForwardRule> = self
            .catalog
            .sessions
            .iter()
            .find(|session| session.id == session_id)
            .map(|session| session.forwards.clone())
            .unwrap_or_default();
        for rule in rules.into_iter().filter(|rule| rule.enabled) {
            // 自动启动不能成为“导入会话后静默开放公网端口”的通道：
            // 公网 bind 一律留待用户在转发面板手动启动并二次确认。
            if is_public_bind_address(forward_bind_host(&rule)) {
                self.set_forward_status(
                    tab_id,
                    rule.id,
                    ForwardState::NeedsConfirmation,
                    Some(AppError::with_detail(
                        ErrorCode::ForwardFailed,
                        "公网监听不会自动启动；请在转发面板手动启动并确认",
                    )),
                );
                continue;
            }
            self.set_forward_status(tab_id, rule.id, ForwardState::Starting, None);
            self.send_command(ForwardCommand::ForwardStart {
                tab_id,
                connection_id,
                rule,
            });
        }
        cx.notify();
    }

    pub(crate) fn forward_panel_mut(&mut self, tab_id: TabId) -> Option<&mut ForwardPanelState> {
        self.workspace
            .tab_mut(tab_id)
            .and_then(|tab| tab.forwards_panel_mut())
    }

    pub(crate) fn toggle_forward_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab_id) = self.workspace.active_tab else {
            return;
        };
        let Some(index) = self.workspace.tab_index(tab_id) else {
            return;
        };
        if self.workspace.tabs[index].forwards_panel().is_some() {
            if let Some(pane) = self.workspace.tabs[index].session_pane_mut() {
                pane.forwards_panel = None;
            }
            cx.notify();
            return;
        }
        if !matches!(
            self.workspace.tabs[index].session.connection,
            Connection::Ssh(_)
        ) {
            self.set_error(rust_i18n::t!("error.forward_requires_ssh").to_string());
            cx.notify();
            return;
        }
        if !matches!(self.workspace.tabs[index].status, TabStatus::Connected) {
            self.set_error(rust_i18n::t!("error.forward_failed").to_string());
            cx.notify();
            return;
        }
        let name = sftp_text_input(window, cx, "", rust_i18n::t!("forward.name").as_ref());
        let bind_host = sftp_text_input(
            window,
            cx,
            "127.0.0.1",
            rust_i18n::t!("forward.bind_host").as_ref(),
        );
        let bind_port = sftp_text_input(
            window,
            cx,
            "8080",
            rust_i18n::t!("forward.bind_port").as_ref(),
        );
        let target_host = sftp_text_input(
            window,
            cx,
            "127.0.0.1",
            rust_i18n::t!("forward.target_host").as_ref(),
        );
        let target_port = sftp_text_input(
            window,
            cx,
            "80",
            rust_i18n::t!("forward.target_port").as_ref(),
        );
        if let Some(pane) = self.workspace.tabs[index].session_pane_mut() {
            pane.forwards_panel = Some(ForwardPanelState {
                name_input: name,
                bind_host_input: bind_host,
                bind_port_input: bind_port,
                target_host_input: target_host,
                target_port_input: target_port,
                kind: ForwardKindChoice::Local,
                error: None,
            });
        }
        cx.notify();
    }

    pub(crate) fn add_forward_rule(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(tab) = self.workspace.tab(tab_id) else {
            return;
        };
        let Some(connection_id) = tab.connection_id else {
            return;
        };
        let session_id = tab.session_id;
        let Some(panel) = tab.forwards_panel() else {
            return;
        };
        let name = panel.name_input.read(cx).value().trim().to_string();
        let bind_host = panel.bind_host_input.read(cx).value().trim().to_string();
        let target_host = panel.target_host_input.read(cx).value().trim().to_string();
        let bind_port: u16 = match panel.bind_port_input.read(cx).value().trim().parse() {
            Ok(port) if port > 0 => port,
            _ => {
                if let Some(panel) = self.forward_panel_mut(tab_id) {
                    panel.error = Some(rust_i18n::t!("forward.error_port").to_string());
                }
                cx.notify();
                return;
            }
        };
        let target_port: u16 = match panel.target_port_input.read(cx).value().trim().parse() {
            Ok(port) if port > 0 => port,
            _ => {
                if let Some(panel) = self.forward_panel_mut(tab_id) {
                    panel.error = Some(rust_i18n::t!("forward.error_port").to_string());
                }
                cx.notify();
                return;
            }
        };
        if bind_host.is_empty() {
            if let Some(panel) = self.forward_panel_mut(tab_id) {
                panel.error = Some(rust_i18n::t!("forward.bind_host").to_string());
            }
            cx.notify();
            return;
        }
        if panel.kind != ForwardKindChoice::Dynamic && target_host.is_empty() {
            if let Some(panel) = self.forward_panel_mut(tab_id) {
                panel.error = Some(rust_i18n::t!("forward.error_target").to_string());
            }
            cx.notify();
            return;
        }
        let kind = match panel.kind {
            ForwardKindChoice::Local => ForwardKind::Local {
                bind: BindAddress {
                    host: bind_host.clone(),
                    port: bind_port,
                },
                target: TargetAddress {
                    host: target_host,
                    port: target_port,
                },
            },
            ForwardKindChoice::Remote => ForwardKind::Remote {
                bind: BindAddress {
                    host: bind_host.clone(),
                    port: bind_port,
                },
                target: TargetAddress {
                    host: target_host,
                    port: target_port,
                },
            },
            ForwardKindChoice::Dynamic => ForwardKind::Dynamic {
                bind: BindAddress {
                    host: bind_host.clone(),
                    port: bind_port,
                },
                socks: SocksOptions::default(),
            },
        };
        let name = if name.is_empty() {
            format!("{bind_host}:{bind_port}")
        } else {
            name
        };
        let rule = ForwardRule::new(name, kind);
        if let Some(session) = self
            .catalog
            .sessions
            .iter_mut()
            .find(|session| session.id == session_id)
        {
            session.forwards.push(rule.clone());
        }
        self.persist_catalog();
        if let Some(panel) = self.forward_panel_mut(tab_id) {
            panel.error = None;
        }

        if is_public_bind_address(&bind_host) {
            self.modals.confirm_forward_any = Some(PendingForwardConfirm {
                tab_id,
                connection_id,
                rule,
            });
            cx.notify();
            return;
        }

        self.set_forward_status(tab_id, rule.id, ForwardState::Starting, None);
        self.send_command(ForwardCommand::ForwardStart {
            tab_id,
            connection_id,
            rule,
        });
        cx.notify();
    }

    pub(crate) fn start_forward_rule(
        &mut self,
        tab_id: TabId,
        rule_id: ForwardId,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.workspace.tab(tab_id) else {
            return;
        };
        let Some(connection_id) = tab.connection_id else {
            return;
        };
        let session_id = tab.session_id;
        let Some(rule) = self
            .catalog
            .sessions
            .iter()
            .find(|session| session.id == session_id)
            .and_then(|session| session.forwards.iter().find(|rule| rule.id == rule_id))
            .cloned()
        else {
            return;
        };

        let bind_host = forward_bind_host(&rule);

        if is_public_bind_address(bind_host) {
            self.modals.confirm_forward_any = Some(PendingForwardConfirm {
                tab_id,
                connection_id,
                rule,
            });
            cx.notify();
            return;
        }

        if let Some(session) = self
            .catalog
            .sessions
            .iter_mut()
            .find(|session| session.id == session_id)
            && let Some(rule) = session.forwards.iter_mut().find(|rule| rule.id == rule_id)
        {
            rule.enabled = true;
        }
        self.persist_catalog();
        self.set_forward_status(tab_id, rule_id, ForwardState::Starting, None);
        self.send_command(ForwardCommand::ForwardStart {
            tab_id,
            connection_id,
            rule,
        });
        cx.notify();
    }

    pub(crate) fn confirm_forward_start(&mut self, cx: &mut Context<Self>) {
        let Some(pending) = self.modals.confirm_forward_any.take() else {
            return;
        };
        let PendingForwardConfirm {
            tab_id,
            connection_id,
            rule,
        } = pending;
        let session_id = self.workspace.tab(tab_id).map(|t| t.session_id);
        if let Some(session_id) = session_id {
            if let Some(session) = self
                .catalog
                .sessions
                .iter_mut()
                .find(|s| s.id == session_id)
                && let Some(r) = session.forwards.iter_mut().find(|r| r.id == rule.id)
            {
                r.enabled = true;
            }
            self.persist_catalog();
        }
        self.set_forward_status(tab_id, rule.id, ForwardState::Starting, None);
        self.send_command(ForwardCommand::ForwardStart {
            tab_id,
            connection_id,
            rule,
        });
        cx.notify();
    }

    pub(crate) fn cancel_forward_confirm(&mut self, cx: &mut Context<Self>) {
        self.modals.confirm_forward_any = None;
        cx.notify();
    }

    pub(crate) fn stop_forward_rule(
        &mut self,
        tab_id: TabId,
        rule_id: ForwardId,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.workspace.tab(tab_id) else {
            return;
        };
        let Some(connection_id) = tab.connection_id else {
            return;
        };
        let session_id = tab.session_id;
        if let Some(session) = self
            .catalog
            .sessions
            .iter_mut()
            .find(|session| session.id == session_id)
            && let Some(rule) = session.forwards.iter_mut().find(|rule| rule.id == rule_id)
        {
            rule.enabled = false;
        }
        self.persist_catalog();
        self.set_forward_status(tab_id, rule_id, ForwardState::Stopped, None);
        self.send_command(ForwardCommand::ForwardStop {
            tab_id,
            connection_id,
            rule_id,
        });
        cx.notify();
    }

    pub(crate) fn delete_forward_rule(
        &mut self,
        tab_id: TabId,
        rule_id: ForwardId,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.workspace.tab(tab_id) else {
            return;
        };
        let connection_id = tab.connection_id;
        let session_id = tab.session_id;
        if let (Some(connection_id), Some(status)) = (
            connection_id,
            self.workspace
                .tab(tab_id)
                .and_then(|tab| tab.session_pane())
                .and_then(|pane| pane.forward_statuses.get(&rule_id)),
        ) && matches!(
            status.state,
            ForwardState::Active { .. } | ForwardState::Starting
        ) {
            self.send_command(ForwardCommand::ForwardStop {
                tab_id,
                connection_id,
                rule_id,
            });
        }
        if let Some(pane) = self
            .workspace
            .tab_mut(tab_id)
            .and_then(|tab| tab.session_pane_mut())
        {
            pane.forward_statuses.remove(&rule_id);
        }
        if let Some(session) = self
            .catalog
            .sessions
            .iter_mut()
            .find(|session| session.id == session_id)
        {
            session.forwards.retain(|rule| rule.id != rule_id);
        }
        self.persist_catalog();
        cx.notify();
    }

    pub(crate) fn on_toggle_forward(
        &mut self,
        _: &keymap::ToggleForward,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_forward_panel(window, cx);
    }
}
