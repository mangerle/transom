//! 批量选择与批量编辑动作。

use super::common::*;
use super::state::{BatchEditForm, BatchUpdates, apply_batch_updates};

impl AppShell {
    pub(crate) fn toggle_selection_mode(&mut self, cx: &mut Context<Self>) {
        self.state.sidebar.selection_mode = !self.state.sidebar.selection_mode;
        self.state.sidebar.clear_selection();
        cx.notify();
    }

    pub(crate) fn toggle_session_selection(
        &mut self,
        session_id: SessionId,
        selected: bool,
        cx: &mut Context<Self>,
    ) {
        if selected {
            self.state.sidebar.selected_sessions.insert(session_id);
        } else {
            self.state.sidebar.selected_sessions.remove(&session_id);
        }
        cx.notify();
    }

    pub(crate) fn select_all_sessions(&mut self, cx: &mut Context<Self>) {
        self.state.sidebar.selected_sessions = self
            .state
            .catalog
            .sessions
            .iter()
            .map(|session| session.id)
            .collect();
        cx.notify();
    }

    pub(crate) fn open_batch_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selected_count = self
            .state
            .sidebar
            .selected_sessions
            .iter()
            .filter(|id| {
                self.state
                    .catalog
                    .sessions
                    .iter()
                    .any(|session| session.id == **id)
            })
            .count();
        if selected_count == 0 {
            self.state.status = Some(rust_i18n::t!("batch.none_selected").to_string());
            cx.notify();
            return;
        }
        self.state.modals.batch_edit = Some(BatchEditForm {
            selected_count,
            change_port: false,
            port: sftp_text_input(window, cx, "22", rust_i18n::t!("form.port").as_ref()),
            change_username: false,
            username: sftp_text_input(window, cx, "", rust_i18n::t!("form.username").as_ref()),
            change_auth: false,
            auth: AuthChoice::Password,
            key_path: sftp_text_input(window, cx, "", rust_i18n::t!("form.key_path").as_ref()),
            save_passphrase: false,
            error: None,
        });
        cx.notify();
    }

    pub(crate) fn save_batch_edit(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.state.modals.batch_edit.as_ref() else {
            return;
        };
        if !form.change_port && !form.change_username && !form.change_auth {
            if let Some(form) = self.state.modals.batch_edit.as_mut() {
                form.error = Some(rust_i18n::t!("batch.error_no_change").to_string());
            }
            cx.notify();
            return;
        }

        let mut port = None;
        if form.change_port {
            match form.port.read(cx).value().trim().parse::<u16>() {
                Ok(value) if value > 0 => port = Some(value),
                _ => {
                    if let Some(form) = self.state.modals.batch_edit.as_mut() {
                        form.error = Some(rust_i18n::t!("batch.error_port").to_string());
                    }
                    cx.notify();
                    return;
                }
            }
        }
        let mut username = None;
        if form.change_username {
            let value = form.username.read(cx).value().trim().to_string();
            if value.is_empty() {
                if let Some(form) = self.state.modals.batch_edit.as_mut() {
                    form.error = Some(rust_i18n::t!("batch.error_username").to_string());
                }
                cx.notify();
                return;
            }
            username = Some(value);
        }
        let auth = if form.change_auth {
            match form.auth {
                AuthChoice::Password => Some(AuthMethod::Password { save: false }),
                AuthChoice::PrivateKey => {
                    let path = form.key_path.read(cx).value().trim().to_string();
                    if path.is_empty() {
                        if let Some(form) = self.state.modals.batch_edit.as_mut() {
                            form.error = Some(rust_i18n::t!("batch.error_key").to_string());
                        }
                        cx.notify();
                        return;
                    }
                    Some(AuthMethod::PrivateKey {
                        path: PathBuf::from(path),
                        // 批量改认证方式不继承各自的证书路径：证书与机器成对下发，
                        // 把某台的证书复制到一批机器上是错的，清空并让用户显式补更稳。
                        cert_path: None,
                        save_passphrase: form.save_passphrase,
                    })
                }
                AuthChoice::Agent => Some(AuthMethod::Agent),
            }
        } else {
            None
        };

        let updates = BatchUpdates {
            port,
            username,
            auth,
        };
        let selected = self.state.sidebar.selected_sessions.clone();
        let (updated, skipped) = apply_batch_updates(&mut self.state.catalog, &selected, &updates);
        self.state.modals.batch_edit = None;
        let saved = self.state.persist_catalog();
        self.state.refresh_tab_sessions();
        self.state.sidebar.selection_mode = false;
        self.state.sidebar.clear_selection();
        if saved {
            self.state.status = Some(
                rust_i18n::t!("batch.updated", updated = updated, skipped = skipped).to_string(),
            );
        }
        cx.notify();
    }
}
