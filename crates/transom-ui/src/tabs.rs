//! 会话增删改查、标签页激活/关闭/轮转与界面语言切换模块。

use crate::app::{AppShell, TabStatus};
use crate::palette::localized_error;
use crate::session_form::SessionForm;
use gpui_kit::component::set_locale;
use gpui_kit::*;
use transom_core::{AppCommand, Connection, ErrorCode, SecretKey, SecretKind, SessionId, TabId};

impl AppShell {
    pub(crate) fn open_new_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.form = Some(SessionForm::new(window, cx, None, None));
        cx.notify();
    }

    pub(crate) fn open_edit_session(
        &mut self,
        session_id: SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self
            .catalog
            .sessions
            .iter()
            .find(|session| session.id == session_id)
            .cloned()
        else {
            return;
        };
        let folder_path = session
            .folder_id
            .and_then(|id| self.catalog.folder_path(id));
        let is_serial = matches!(session.connection, Connection::Serial(_));
        self.form = Some(SessionForm::new(window, cx, Some(session), folder_path));
        if is_serial {
            self.refresh_serial_ports(window, cx);
        }
        cx.notify();
    }

    pub(crate) fn save_session_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = self.form.as_mut() else {
            return;
        };
        let editing = form.base.is_some();
        let save_password = form.save_password;
        let save_passphrase = form.save_passphrase;
        let built = match form.build_session(cx) {
            Ok(built) => built,
            Err(error) => {
                form.error = Some(error);
                cx.notify();
                return;
            }
        };
        self.form = None;

        let mut built = built;
        built.session.folder_id = built
            .folder_path
            .as_deref()
            .and_then(|path| self.catalog.ensure_folder_path(path));
        let password_key = SecretKey::new(built.session.id, SecretKind::Password);
        let passphrase_key = SecretKey::new(built.session.id, SecretKind::KeyPassphrase);
        if save_password {
            if let Some(password) = &built.password
                && let Err(error) = self
                    .services
                    .secret_store
                    .set(&password_key, password.as_str())
            {
                self.error = Some(localized_error(&error));
            }
        } else if let Err(error) = self.services.secret_store.delete(&password_key)
            && error.code != ErrorCode::SecretStoreUnavailable
        {
            self.error = Some(localized_error(&error));
        }
        if save_passphrase {
            if let Some(passphrase) = &built.key_passphrase
                && let Err(error) = self
                    .services
                    .secret_store
                    .set(&passphrase_key, passphrase.as_str())
            {
                self.error = Some(localized_error(&error));
            }
        } else if let Err(error) = self.services.secret_store.delete(&passphrase_key)
            && error.code != ErrorCode::SecretStoreUnavailable
        {
            self.error = Some(localized_error(&error));
        }

        let session_id = built.session.id;
        if editing {
            if let Some(existing) = self
                .catalog
                .sessions
                .iter_mut()
                .find(|session| session.id == session_id)
            {
                *existing = built.session;
            }
        } else {
            self.catalog.sessions.push(built.session);
        }
        self.persist_catalog();
        if !editing {
            self.connect_session(session_id, window, cx);
        }
        cx.notify();
    }

    pub(crate) fn delete_session_form(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.form.take() else {
            return;
        };
        let Some(base) = form.base else {
            return;
        };
        self.delete_session_by_id(base.id, cx);
    }

    pub(crate) fn delete_session_by_id(&mut self, session_id: SessionId, cx: &mut Context<Self>) {
        self.catalog
            .sessions
            .retain(|session| session.id != session_id);
        let removed_tabs: Vec<TabId> = self
            .tabs
            .iter()
            .filter(|tab| tab.session_id == session_id)
            .map(|tab| tab.id)
            .collect();
        for tab_id in removed_tabs {
            let _ = self
                .services
                .command_tx
                .send(AppCommand::CloseTab { tab_id });
        }
        self.tabs.retain(|tab| tab.session_id != session_id);
        let _ = self
            .services
            .secret_store
            .delete(&SecretKey::new(session_id, SecretKind::Password));
        let _ = self
            .services
            .secret_store
            .delete(&SecretKey::new(session_id, SecretKind::KeyPassphrase));
        self.persist_catalog();
        cx.notify();
    }

    pub(crate) fn duplicate_session(&mut self, session_id: SessionId, cx: &mut Context<Self>) {
        if let Some(mut session) = self
            .catalog
            .sessions
            .iter()
            .find(|s| s.id == session_id)
            .cloned()
        {
            session.id = SessionId::new();
            session.name = format!("{} (Copy)", session.name);
            self.catalog.sessions.push(session);
            self.persist_catalog();
            cx.notify();
        }
    }

    pub(crate) fn close_other_tabs(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let other_tab_ids: Vec<TabId> = self
            .tabs
            .iter()
            .map(|t| t.id)
            .filter(|&id| id != tab_id)
            .collect();
        for other in other_tab_ids {
            self.close_tab(other, window, cx);
        }
    }

    pub(crate) fn reconnect_tab(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.tabs.iter().position(|tab| tab.id == tab_id) else {
            return;
        };
        let session_id = self.tabs[index].session_id;
        let session = self
            .catalog
            .sessions
            .iter()
            .find(|session| session.id == session_id)
            .cloned()
            .unwrap_or_else(|| self.tabs[index].session.clone());
        let tab = &mut self.tabs[index];
        tab.terminal = None;
        tab.sftp = None;
        tab._subscription = None;
        tab.connection_id = None;
        tab.forward_statuses.clear();
        tab.network_traffic = None;
        tab.status = TabStatus::Connecting;
        tab.title = session.name.clone();
        self.active_tab = Some(tab_id);
        self.start_connection(tab_id, session, window, cx);
    }

    pub(crate) fn activate_tab(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.active_tab = Some(tab_id);
        if let Some(terminal) = self
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| tab.terminal.as_ref())
        {
            let focus_handle = terminal.read(cx).focus_handle().clone();
            window.focus(&focus_handle, cx);
        }
        cx.notify();
    }

    pub(crate) fn close_tab(&mut self, tab_id: TabId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.tabs.iter().position(|tab| tab.id == tab_id) else {
            return;
        };
        let _ = self
            .services
            .command_tx
            .send(AppCommand::CloseTab { tab_id });
        self.tabs.remove(index);
        if self.active_tab == Some(tab_id) {
            self.active_tab = self
                .tabs
                .get(index.saturating_sub(1))
                .or_else(|| self.tabs.first())
                .map(|tab| tab.id);
            if let Some(active) = self.active_tab {
                self.activate_tab(active, window, cx);
            }
        }
        if self.tabs.is_empty() && self.settings.window.close_window_on_all_tabs_closed {
            self.close_app(window, cx);
            return;
        }
        cx.notify();
    }

    pub(crate) fn remove_tab(&mut self, tab_id: TabId) {
        self.tabs.retain(|tab| tab.id != tab_id);
        if self.active_tab == Some(tab_id) {
            self.active_tab = self.tabs.first().map(|tab| tab.id);
        }
    }

    pub(crate) fn cycle_tab(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.is_empty() {
            return;
        }
        let current = self
            .active_tab
            .and_then(|id| self.tabs.iter().position(|tab| tab.id == id))
            .unwrap_or(0);
        let delta = if forward { 1 } else { self.tabs.len() - 1 };
        let next = (current + delta) % self.tabs.len();
        let tab_id = self.tabs[next].id;
        self.activate_tab(tab_id, window, cx);
    }

    pub(crate) fn toggle_language(&mut self, cx: &mut Context<Self>) {
        self.settings.language = if self.settings.language == "zh-CN" {
            "en".to_string()
        } else {
            "zh-CN".to_string()
        };
        set_locale(&self.settings.language);
        if let Err(error) = self.services.settings_store.save(&self.settings) {
            self.error = Some(localized_error(&error));
        }
        cx.notify();
    }
}
