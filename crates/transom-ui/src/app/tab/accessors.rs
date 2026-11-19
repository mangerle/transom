//! 标签类型的安全访问器。

use super::{SessionPaneState, SessionTab, TabContent, WorkspaceTab};
use crate::app::SessionMenuData;
use gpui_kit::Entity;
use transom_core::{Connection, ConnectionId, Session, SessionId};
use transom_terminal::TerminalView;

impl WorkspaceTab {
    pub(crate) fn is_player(&self) -> bool {
        matches!(self.content, TabContent::Player(_))
    }

    pub(crate) fn is_monitor(&self) -> bool {
        matches!(self.content, TabContent::Monitor(_))
    }

    pub(crate) fn session_tab(&self) -> Option<&SessionTab> {
        match &self.content {
            TabContent::Session(session) => Some(session),
            _ => None,
        }
    }

    pub(crate) fn session_tab_mut(&mut self) -> Option<&mut SessionTab> {
        match &mut self.content {
            TabContent::Session(session) => Some(session),
            _ => None,
        }
    }

    pub(crate) fn session(&self) -> Option<&Session> {
        self.session_tab().map(|session| &session.session)
    }

    pub(crate) fn session_id(&self) -> Option<SessionId> {
        self.session_tab().map(|session| session.session_id)
    }

    pub(crate) fn connection_id(&self) -> Option<ConnectionId> {
        self.session_tab().and_then(|session| session.connection_id)
    }

    pub(crate) fn set_connection_id(&mut self, connection_id: Option<ConnectionId>) {
        if let Some(session) = self.session_tab_mut() {
            session.connection_id = connection_id;
        }
    }

    pub(crate) fn cwd(&self) -> Option<&str> {
        self.session_tab()
            .and_then(|session| session.cwd.as_deref())
    }

    pub(crate) fn set_cwd(&mut self, cwd: Option<String>) {
        if let Some(session) = self.session_tab_mut() {
            session.cwd = cwd;
        }
    }

    pub(crate) fn session_pane(&self) -> Option<&SessionPaneState> {
        self.session_tab().map(|session| &session.pane)
    }

    pub(crate) fn session_pane_mut(&mut self) -> Option<&mut SessionPaneState> {
        self.session_tab_mut().map(|session| &mut session.pane)
    }

    pub(crate) fn terminal(&self) -> Option<Entity<TerminalView>> {
        self.session_pane().and_then(|pane| pane.terminal.clone())
    }

    pub(crate) fn sftp_view(&self) -> Option<Entity<crate::sftp::SftpPanelView>> {
        self.session_pane().and_then(|pane| pane.sftp_view.clone())
    }

    pub(crate) fn forwards_view(&self) -> Option<Entity<crate::forward::ForwardView>> {
        self.session_pane()
            .and_then(|pane| pane.forwards_view.clone())
    }

    pub(crate) fn zmodem(&self) -> Option<&crate::zmodem::ZmodemState> {
        self.session_pane().and_then(|pane| pane.zmodem.as_ref())
    }

    pub(crate) fn system_stats(&self) -> Option<&transom_core::SystemStats> {
        self.session_pane()
            .and_then(|pane| pane.system_stats.as_ref())
    }

    pub(crate) fn network_traffic(&self) -> Option<&transom_core::NetworkTraffic> {
        self.session_pane()
            .and_then(|pane| pane.network_traffic.as_ref())
    }

    pub(crate) fn display_name(&self) -> String {
        if let Some(session) = self.session() {
            let name = session.name.trim();
            if !name.is_empty() {
                return name.to_string();
            }
            return crate::connection::display_compact(&session.connection);
        }
        self.title.clone()
    }

    pub(crate) fn tab_label(&self) -> String {
        let runtime: String = self
            .title
            .chars()
            .map(|character| {
                if character.is_control() {
                    ' '
                } else {
                    character
                }
            })
            .collect();
        let runtime = runtime.trim();
        if runtime.is_empty() {
            self.display_name()
        } else {
            runtime.to_string()
        }
    }

    pub(crate) fn session_menu(&self) -> Option<SessionMenuData> {
        let session = self.session()?;
        Some(SessionMenuData {
            session_id: session.id,
            is_local: matches!(session.connection, Connection::Local(_)),
            host: crate::connection::session_host(session).map(str::to_string),
        })
    }
}
