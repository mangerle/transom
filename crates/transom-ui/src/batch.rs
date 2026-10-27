//! 会话多选与批量属性更新。

use crate::app::{AppShell, BatchEditForm};
use crate::palette::{Palette, sftp_text_input};
use crate::session_form::AuthChoice;
use gpui_kit::base::{Selectable, h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::Input;
use gpui_kit::component::switch::Switch;
use gpui_kit::*;
use std::collections::HashSet;
use std::path::PathBuf;
use transom_core::{AuthMethod, Connection, SessionCatalog, SessionId};

/// 批量更新值；`None` 表示该属性保持不变。
struct BatchUpdates {
    port: Option<u16>,
    username: Option<String>,
    auth: Option<AuthMethod>,
}

/// 将批量更新应用到选中的 SSH 会话，返回（已更新，已跳过）数量。
///
/// 本地与串口会话没有端口/用户名/SSH 认证概念，统一跳过而不是静默修改。
fn apply_batch_updates(
    catalog: &mut SessionCatalog,
    selected: &HashSet<SessionId>,
    updates: &BatchUpdates,
) -> (usize, usize) {
    let mut updated = 0;
    let mut skipped = 0;
    for session in &mut catalog.sessions {
        if !selected.contains(&session.id) {
            continue;
        }
        let Connection::Ssh(ssh) = &mut session.connection else {
            skipped += 1;
            continue;
        };
        if let Some(port) = updates.port {
            ssh.port = port;
        }
        if let Some(username) = &updates.username {
            ssh.username = username.clone();
        }
        if let Some(auth) = &updates.auth {
            ssh.auth = auth.clone();
        }
        updated += 1;
    }
    (updated, skipped)
}

impl AppShell {
    pub(crate) fn toggle_selection_mode(&mut self, cx: &mut Context<Self>) {
        self.selection_mode = !self.selection_mode;
        self.selected_sessions.clear();
        cx.notify();
    }

    pub(crate) fn toggle_session_selection(
        &mut self,
        session_id: SessionId,
        selected: bool,
        cx: &mut Context<Self>,
    ) {
        if selected {
            self.selected_sessions.insert(session_id);
        } else {
            self.selected_sessions.remove(&session_id);
        }
        cx.notify();
    }

    pub(crate) fn select_all_sessions(&mut self, cx: &mut Context<Self>) {
        self.selected_sessions = self
            .catalog
            .sessions
            .iter()
            .map(|session| session.id)
            .collect();
        cx.notify();
    }

    pub(crate) fn open_batch_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selected_count = self
            .selected_sessions
            .iter()
            .filter(|id| {
                self.catalog
                    .sessions
                    .iter()
                    .any(|session| session.id == **id)
            })
            .count();
        if selected_count == 0 {
            self.status = Some(rust_i18n::t!("batch.none_selected").to_string());
            cx.notify();
            return;
        }
        self.modals.batch_edit = Some(BatchEditForm {
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
        let Some(form) = self.modals.batch_edit.as_ref() else {
            return;
        };
        if !form.change_port && !form.change_username && !form.change_auth {
            if let Some(form) = self.modals.batch_edit.as_mut() {
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
                    if let Some(form) = self.modals.batch_edit.as_mut() {
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
                if let Some(form) = self.modals.batch_edit.as_mut() {
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
                        if let Some(form) = self.modals.batch_edit.as_mut() {
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
        let selected = self.selected_sessions.clone();
        let (updated, skipped) = apply_batch_updates(&mut self.catalog, &selected, &updates);
        self.modals.batch_edit = None;
        let saved = self.persist_catalog();
        self.refresh_tab_sessions();
        self.selection_mode = false;
        self.selected_sessions.clear();
        if saved {
            self.status = Some(
                rust_i18n::t!("batch.updated", updated = updated, skipped = skipped).to_string(),
            );
        }
        cx.notify();
    }

    pub(crate) fn render_batch_edit(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let Some(form) = self.modals.batch_edit.as_ref() else {
            return div().into_any_element();
        };
        let selected_count = form.selected_count;
        let change_port = form.change_port;
        let change_username = form.change_username;
        let change_auth = form.change_auth;
        let auth = form.auth;
        let save_passphrase = form.save_passphrase;
        let error = form.error.clone();

        let port_row = h_flex()
            .gap_2()
            .items_center()
            .child(
                Checkbox::new("batch-change-port")
                    .small()
                    .checked(change_port)
                    .on_click(cx.listener(|this, &checked, _, cx| {
                        if let Some(form) = this.modals.batch_edit.as_mut() {
                            form.change_port = checked;
                        }
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .w(px(72.0))
                    .text_sm()
                    .child(rust_i18n::t!("form.port").to_string()),
            )
            .child(div().flex_1().child(Input::new(&form.port)));

        let username_row = h_flex()
            .gap_2()
            .items_center()
            .child(
                Checkbox::new("batch-change-username")
                    .small()
                    .checked(change_username)
                    .on_click(cx.listener(|this, &checked, _, cx| {
                        if let Some(form) = this.modals.batch_edit.as_mut() {
                            form.change_username = checked;
                        }
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .w(px(72.0))
                    .text_sm()
                    .child(rust_i18n::t!("form.username").to_string()),
            )
            .child(div().flex_1().child(Input::new(&form.username)));

        let auth_buttons = h_flex()
            .gap_2()
            .child(
                Button::new("batch-auth-password")
                    .small()
                    .label(rust_i18n::t!("form.auth_password").to_string())
                    .selected(auth == AuthChoice::Password)
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(form) = this.modals.batch_edit.as_mut() {
                            form.auth = AuthChoice::Password;
                        }
                        cx.notify();
                    })),
            )
            .child(
                Button::new("batch-auth-key")
                    .small()
                    .label(rust_i18n::t!("form.auth_key").to_string())
                    .selected(auth == AuthChoice::PrivateKey)
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(form) = this.modals.batch_edit.as_mut() {
                            form.auth = AuthChoice::PrivateKey;
                        }
                        cx.notify();
                    })),
            )
            .child(
                Button::new("batch-auth-agent")
                    .small()
                    .label(rust_i18n::t!("form.auth_agent").to_string())
                    .selected(auth == AuthChoice::Agent)
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(form) = this.modals.batch_edit.as_mut() {
                            form.auth = AuthChoice::Agent;
                        }
                        cx.notify();
                    })),
            );

        let mut auth_section = v_flex().gap_2().child(
            h_flex()
                .gap_2()
                .items_center()
                .child(
                    Checkbox::new("batch-change-auth")
                        .small()
                        .checked(change_auth)
                        .on_click(cx.listener(|this, &checked, _, cx| {
                            if let Some(form) = this.modals.batch_edit.as_mut() {
                                form.change_auth = checked;
                            }
                            cx.notify();
                        })),
                )
                .child(
                    div()
                        .w(px(72.0))
                        .text_sm()
                        .child(rust_i18n::t!("form.auth").to_string()),
                )
                .child(auth_buttons),
        );
        if change_auth {
            match auth {
                AuthChoice::Password => {
                    auth_section = auth_section.child(
                        div()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child(rust_i18n::t!("batch.password_hint").to_string()),
                    );
                }
                AuthChoice::PrivateKey => {
                    auth_section = auth_section.child(Input::new(&form.key_path)).child(
                        h_flex()
                            .justify_between()
                            .items_center()
                            .child(
                                div()
                                    .text_sm()
                                    .child(rust_i18n::t!("form.save_passphrase").to_string()),
                            )
                            .child(
                                Switch::new("batch-save-passphrase")
                                    .small()
                                    .checked(save_passphrase)
                                    .on_click(cx.listener(|this, &checked, _, cx| {
                                        if let Some(form) = this.modals.batch_edit.as_mut() {
                                            form.save_passphrase = checked;
                                        }
                                        cx.notify();
                                    })),
                            ),
                    );
                }
                AuthChoice::Agent => {}
            }
        }

        let mut content = v_flex()
            .gap_3()
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("batch.hint", count = selected_count).to_string()),
            )
            .child(port_row)
            .child(username_row)
            .child(auth_section);
        if let Some(error) = error {
            content = content.child(div().text_sm().text_color(p.danger).child(error));
        }

        let footer = h_flex()
            .gap_2()
            .child(
                Button::new("cancel-batch-edit")
                    .ghost()
                    .label(rust_i18n::t!("common.cancel").to_string())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.modals.batch_edit = None;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("apply-batch-edit")
                    .primary()
                    .label(rust_i18n::t!("batch.apply").to_string())
                    .on_click(cx.listener(|this, _, _, cx| this.save_batch_edit(cx))),
            );

        let dialog = Self::dialog_frame(
            "batch-edit-modal",
            rust_i18n::t!("batch.title").to_string(),
            Some(rust_i18n::t!("batch.apply_hint").to_string()),
            content,
            Some(footer),
            px(560.0),
            p,
        );
        Self::overlay(dialog, p)
    }
}

#[cfg(test)]
mod tests {
    use super::BatchUpdates;
    use super::apply_batch_updates;
    use std::collections::HashSet;
    use transom_core::{
        AuthMethod, Connection, LocalConnection, SerialConnection, Session, SessionCatalog,
        SshConnection,
    };

    fn ssh_session(name: &str) -> Session {
        Session::new(
            name,
            Connection::Ssh(SshConnection {
                host: "example.com".into(),
                port: 22,
                username: "old".into(),
                auth: AuthMethod::Agent,
                options: Default::default(),
                proxy_jump: None,
            }),
        )
    }

    #[test]
    fn updates_only_selected_ssh_sessions() {
        let mut catalog = SessionCatalog::default();
        let ssh = ssh_session("ssh");
        let ssh_id = ssh.id;
        let local = Session::new("local", Connection::Local(LocalConnection::default()));
        let local_id = local.id;
        let serial = Session::new(
            "serial",
            Connection::Serial(SerialConnection::new("/dev/ttyS0", 9600)),
        );
        let serial_id = serial.id;
        catalog.sessions = vec![ssh, local, serial];
        let selected: HashSet<_> = [ssh_id, local_id, serial_id].into_iter().collect();

        let updates = BatchUpdates {
            port: Some(2222),
            username: Some("deploy".into()),
            auth: Some(AuthMethod::Agent),
        };
        let (updated, skipped) = apply_batch_updates(&mut catalog, &selected, &updates);
        assert_eq!((updated, skipped), (1, 2));

        let Connection::Ssh(ssh) = &catalog.sessions[0].connection else {
            panic!("应为 SSH 会话");
        };
        assert_eq!(ssh.port, 2222);
        assert_eq!(ssh.username, "deploy");
    }

    #[test]
    fn unselected_sessions_keep_their_properties() {
        let mut catalog = SessionCatalog::default();
        let selected = ssh_session("selected");
        let untouched = ssh_session("untouched");
        let selected_id = selected.id;
        catalog.sessions = vec![selected, untouched];
        let selected_ids = HashSet::from([selected_id]);

        let updates = BatchUpdates {
            port: Some(2022),
            username: None,
            auth: None,
        };
        apply_batch_updates(&mut catalog, &selected_ids, &updates);
        let Connection::Ssh(ssh) = &catalog.sessions[1].connection else {
            panic!("应为 SSH 会话");
        };
        assert_eq!(ssh.port, 22);
    }
}
