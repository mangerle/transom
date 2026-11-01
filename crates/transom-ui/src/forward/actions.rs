//! 端口转发规则管理与启停控制方法。

use super::{
    ForwardKindChoice, ForwardPanelState, PendingForwardConfirm, forward_bind_host,
    is_public_bind_address,
};
use crate::app::{AppShell, AppState, TabStatus};
use crate::keymap;
use crate::palette::sftp_text_input;
use gpui_kit::*;
use transom_core::{
    AppError, Connection, ConnectionId, ErrorCode, ForwardCommand, ForwardId, ForwardRule,
    ForwardState, ForwardStatus, SessionId, TabId,
};

impl AppState {
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
}

impl AppShell {
    /// 处理转发面板发来的意图。
    ///
    /// 只用事件携带的数据，不回读面板：事件派发阶段面板仍被租借。
    fn on_forward_panel_event(
        &mut self,
        event: &crate::forward::ForwardPanelEvent,
        cx: &mut Context<Self>,
    ) {
        use crate::forward::ForwardPanelEvent;
        match event {
            ForwardPanelEvent::SubmitRule { tab_id, rule } => {
                self.submit_forward_rule(*tab_id, rule.clone(), cx);
            }
            ForwardPanelEvent::StartRule { tab_id, rule_id } => {
                self.start_forward_rule(*tab_id, *rule_id, cx);
            }
            ForwardPanelEvent::StopRule { tab_id, rule_id } => {
                self.stop_forward_rule(*tab_id, *rule_id, cx);
            }
            ForwardPanelEvent::DeleteRule { tab_id, rule_id } => {
                self.delete_forward_rule(*tab_id, *rule_id, cx);
            }
            ForwardPanelEvent::ClosePanel => {
                self.state.workspace.close_forward_panel();
                cx.notify();
            }
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
            .state
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
                self.state.set_forward_status(
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
            self.state
                .set_forward_status(tab_id, rule.id, ForwardState::Starting, None);
            self.state.send_command(ForwardCommand::ForwardStart {
                tab_id,
                connection_id,
                rule,
            });
        }
        cx.notify();
    }

    pub(crate) fn toggle_forward_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab_id) = self.state.workspace.active_tab else {
            return;
        };
        let Some(index) = self.state.workspace.tab_index(tab_id) else {
            return;
        };
        if self.state.workspace.tabs[index].forwards_view().is_some() {
            if let Some(pane) = self.state.workspace.tabs[index].session_pane_mut() {
                pane.forwards_view = None;
            }
            cx.notify();
            return;
        }
        if !matches!(
            self.state.workspace.tabs[index].session.connection,
            Connection::Ssh(_)
        ) {
            self.state
                .set_error(rust_i18n::t!("error.forward_requires_ssh").to_string());
            cx.notify();
            return;
        }
        if !matches!(
            self.state.workspace.tabs[index].status,
            TabStatus::Connected
        ) {
            self.state
                .set_error(rust_i18n::t!("error.forward_failed").to_string());
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
        let view = cx.new(|_| {
            super::ForwardView::new(
                ForwardPanelState {
                    name_input: name,
                    bind_host_input: bind_host,
                    bind_port_input: bind_port,
                    target_host_input: target_host,
                    target_port_input: target_port,
                    kind: ForwardKindChoice::Local,
                    error: None,
                },
                tab_id,
            )
        });
        let subscription = cx.subscribe(
            &view,
            |shell, _view, event: &crate::forward::ForwardPanelEvent, cx| {
                shell.on_forward_panel_event(event, cx);
            },
        );
        self.state
            .tasks
            .subscriptions
            .forward
            .insert(tab_id, subscription);
        if let Some(pane) = self.state.workspace.tabs[index].session_pane_mut() {
            pane.forwards_view = Some(view);
        }
        cx.notify();
    }

    /// 面板校验通过后提交规则：落库、必要时要求公网确认，否则直接下发启动命令。
    pub(crate) fn submit_forward_rule(
        &mut self,
        tab_id: TabId,
        rule: ForwardRule,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.state.workspace.tab(tab_id) else {
            return;
        };
        let Some(connection_id) = tab.connection_id else {
            return;
        };
        let session_id = tab.session_id;

        if let Some(session) = self
            .state
            .catalog
            .sessions
            .iter_mut()
            .find(|session| session.id == session_id)
        {
            session.forwards.push(rule.clone());
        }
        self.state.persist_catalog();

        // 公网监听必须二次确认：不能因为点了一次「添加」就把端口开到公网。
        if is_public_bind_address(forward_bind_host(&rule)) {
            self.state.modals.tools.confirm_forward_any = Some(PendingForwardConfirm {
                tab_id,
                connection_id,
                rule,
            });
            cx.notify();
            return;
        }

        self.state
            .set_forward_status(tab_id, rule.id, ForwardState::Starting, None);
        self.state.send_command(ForwardCommand::ForwardStart {
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
        let Some(tab) = self.state.workspace.tab(tab_id) else {
            return;
        };
        let Some(connection_id) = tab.connection_id else {
            return;
        };
        let session_id = tab.session_id;
        let Some(rule) = self
            .state
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
            self.state.modals.tools.confirm_forward_any = Some(PendingForwardConfirm {
                tab_id,
                connection_id,
                rule,
            });
            cx.notify();
            return;
        }

        if let Some(session) = self
            .state
            .catalog
            .sessions
            .iter_mut()
            .find(|session| session.id == session_id)
            && let Some(rule) = session.forwards.iter_mut().find(|rule| rule.id == rule_id)
        {
            rule.enabled = true;
        }
        self.state.persist_catalog();
        self.state
            .set_forward_status(tab_id, rule_id, ForwardState::Starting, None);
        self.state.send_command(ForwardCommand::ForwardStart {
            tab_id,
            connection_id,
            rule,
        });
        cx.notify();
    }

    pub(crate) fn confirm_forward_start(&mut self, cx: &mut Context<Self>) {
        let Some(pending) = self.state.modals.tools.confirm_forward_any.take() else {
            return;
        };
        let PendingForwardConfirm {
            tab_id,
            connection_id,
            rule,
        } = pending;
        let session_id = self.state.workspace.tab(tab_id).map(|t| t.session_id);
        if let Some(session_id) = session_id {
            if let Some(session) = self
                .state
                .catalog
                .sessions
                .iter_mut()
                .find(|s| s.id == session_id)
                && let Some(r) = session.forwards.iter_mut().find(|r| r.id == rule.id)
            {
                r.enabled = true;
            }
            self.state.persist_catalog();
        }
        self.state
            .set_forward_status(tab_id, rule.id, ForwardState::Starting, None);
        self.state.send_command(ForwardCommand::ForwardStart {
            tab_id,
            connection_id,
            rule,
        });
        cx.notify();
    }

    pub(crate) fn cancel_forward_confirm(&mut self, cx: &mut Context<Self>) {
        self.state.modals.tools.confirm_forward_any = None;
        cx.notify();
    }

    pub(crate) fn stop_forward_rule(
        &mut self,
        tab_id: TabId,
        rule_id: ForwardId,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.state.workspace.tab(tab_id) else {
            return;
        };
        let Some(connection_id) = tab.connection_id else {
            return;
        };
        let session_id = tab.session_id;
        if let Some(session) = self
            .state
            .catalog
            .sessions
            .iter_mut()
            .find(|session| session.id == session_id)
            && let Some(rule) = session.forwards.iter_mut().find(|rule| rule.id == rule_id)
        {
            rule.enabled = false;
        }
        self.state.persist_catalog();
        self.state
            .set_forward_status(tab_id, rule_id, ForwardState::Stopped, None);
        self.state.send_command(ForwardCommand::ForwardStop {
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
        let Some(tab) = self.state.workspace.tab(tab_id) else {
            return;
        };
        let connection_id = tab.connection_id;
        let session_id = tab.session_id;
        if let (Some(connection_id), Some(status)) = (
            connection_id,
            self.state
                .workspace
                .tab(tab_id)
                .and_then(|tab| tab.session_pane())
                .and_then(|pane| pane.forward_statuses.get(&rule_id)),
        ) && matches!(
            status.state,
            ForwardState::Active { .. } | ForwardState::Starting
        ) {
            self.state.send_command(ForwardCommand::ForwardStop {
                tab_id,
                connection_id,
                rule_id,
            });
        }
        if let Some(pane) = self
            .state
            .workspace
            .tab_mut(tab_id)
            .and_then(|tab| tab.session_pane_mut())
        {
            pane.forward_statuses.remove(&rule_id);
        }
        if let Some(session) = self
            .state
            .catalog
            .sessions
            .iter_mut()
            .find(|session| session.id == session_id)
        {
            session.forwards.retain(|rule| rule.id != rule_id);
        }
        self.state.persist_catalog();
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
