//! 会话连接建立、SSH 配置导入与密码/多因子认证交互模块。

use crate::app::{AppShell, PasswordPrompt, PendingConnect, TabStatus, TerminalTab};
use crate::keymap;
use crate::palette::{Palette, localized_error, password_input};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use std::collections::HashMap;
use transom_core::{
    AppCommand, AuthMethod, Connection, ConnectionId, ConnectionSecrets, LocalConnection,
    SecretKey, SecretKind, Session, SessionId, TabId, TerminalSize, parse_ssh_config,
};
use zeroize::Zeroizing;

impl AppShell {
    pub(crate) fn on_new_local_terminal(
        &mut self,
        _: &keymap::NewLocalTerminal,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_local_terminal(window, cx);
    }

    pub(crate) fn on_quick_open(
        &mut self,
        _: &keymap::QuickOpen,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_quick_open(window, cx);
    }

    pub(crate) fn toggle_quick_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_quick_open = !self.show_quick_open;
        if self.show_quick_open {
            self.quick_open_state.update(cx, |state, cx| {
                state.set_query("", window, cx);
                state.focus(window, cx);
            });
        }
        cx.notify();
    }

    pub(crate) fn open_local_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let tab_id = TabId::new();
        let session = Session::new(
            rust_i18n::t!("app.new_local").to_string(),
            Connection::Local(LocalConnection::default()),
        );
        let session_id = session.id;
        self.tabs.push(TerminalTab {
            id: tab_id,
            session_id,
            session: session.clone(),
            connection_id: None,
            title: session.name.clone(),
            status: TabStatus::Connecting,
            terminal: None,
            sftp: None,
            forwards_panel: None,
            forward_statuses: HashMap::new(),
            system_stats: None,
            _subscription: None,
        });
        self.active_tab = Some(tab_id);
        self.start_connection(tab_id, session, window, cx);
    }

    pub(crate) fn import_ssh_config(&mut self, cx: &mut Context<Self>) {
        let ssh_config_path =
            directories::BaseDirs::new().map(|dirs| dirs.home_dir().join(".ssh").join("config"));

        let Some(path) = ssh_config_path else {
            self.error = Some(rust_i18n::t!("app.import_none").to_string());
            cx.notify();
            return;
        };

        if !path.exists() {
            self.error = Some(rust_i18n::t!("app.import_none").to_string());
            cx.notify();
            return;
        }

        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                self.error = Some(e.to_string());
                cx.notify();
                return;
            }
        };

        let parsed = parse_ssh_config(&content);
        if parsed.is_empty() {
            self.error = Some(rust_i18n::t!("app.import_none").to_string());
            cx.notify();
            return;
        }

        let mut added_count = 0;
        for session in parsed {
            if !self.sessions.iter().any(|s| s.name == session.name) {
                self.sessions.push(session);
                added_count += 1;
            }
        }

        if added_count > 0 {
            if let Err(error) = self.services.session_store.save(&self.sessions) {
                self.error = Some(localized_error(&error));
            } else {
                self.status =
                    Some(rust_i18n::t!("app.import_success", count = added_count).to_string());
            }
        } else {
            self.status = Some(rust_i18n::t!("app.import_none").to_string());
        }
        cx.notify();
    }

    pub(crate) fn connect_session(
        &mut self,
        session_id: SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(tab_id) = self
            .tabs
            .iter()
            .find(|tab| tab.session_id == session_id)
            .map(|tab| tab.id)
        {
            self.activate_tab(tab_id, window, cx);
            return;
        }
        let Some(session) = self
            .sessions
            .iter()
            .find(|session| session.id == session_id)
            .cloned()
        else {
            return;
        };

        let tab_id = TabId::new();
        self.tabs.push(TerminalTab {
            id: tab_id,
            session_id,
            session: session.clone(),
            connection_id: None,
            title: session.name.clone(),
            status: TabStatus::Connecting,
            terminal: None,
            sftp: None,
            forwards_panel: None,
            forward_statuses: HashMap::new(),
            system_stats: None,
            _subscription: None,
        });
        self.active_tab = Some(tab_id);
        self.start_connection(tab_id, session, window, cx);
    }

    pub(crate) fn start_connection(
        &mut self,
        tab_id: TabId,
        session: Session,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let connection_id = ConnectionId::new();
        if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id) {
            tab.connection_id = Some(connection_id);
            tab.sftp = None;
            tab.forward_statuses.clear();
        }
        let mut secrets = ConnectionSecrets::default();
        let mut needs_password = false;

        match &session.connection {
            Connection::Ssh(ssh) => match &ssh.auth {
                AuthMethod::Password { .. } => {
                    let key = SecretKey::new(session.id, SecretKind::Password);
                    match self.services.secret_store.get(&key) {
                        Ok(Some(secret)) => secrets.password = Some(secret),
                        Ok(None) => needs_password = true,
                        Err(error) => {
                            self.error = Some(localized_error(&error));
                            needs_password = true;
                        }
                    }
                }
                AuthMethod::PrivateKey {
                    save_passphrase, ..
                } => {
                    if *save_passphrase {
                        let key = SecretKey::new(session.id, SecretKind::KeyPassphrase);
                        match self.services.secret_store.get(&key) {
                            Ok(Some(secret)) => secrets.key_passphrase = Some(secret),
                            Ok(None) => {}
                            Err(error) => self.error = Some(localized_error(&error)),
                        }
                    }
                }
                AuthMethod::Agent => {}
                _ => {}
            },
            Connection::Local(_) => {}
            _ => {
                self.error = Some(rust_i18n::t!("error.configuration").to_string());
                return;
            }
        }

        if needs_password {
            let password_prompt = PasswordPrompt {
                input: password_input(window, cx),
                error: None,
            };
            self.pending_connect = Some(PendingConnect {
                tab_id,
                connection_id,
                session,
                secrets,
            });
            self.password_prompt = Some(password_prompt);
        } else {
            self.send_connect(tab_id, connection_id, session, secrets, cx);
        }
        cx.notify();
    }

    pub(crate) fn send_connect(
        &mut self,
        tab_id: TabId,
        connection_id: ConnectionId,
        session: Session,
        secrets: ConnectionSecrets,
        cx: &mut Context<Self>,
    ) {
        let size = TerminalSize::new(120, 32);
        let command = AppCommand::Connect {
            tab_id,
            connection_id,
            session: Box::new(session),
            secrets,
            size,
        };
        if let Err(error) = self.services.command_tx.send(command) {
            self.error = Some(error.to_string());
            self.remove_tab(tab_id);
        }
        cx.notify();
    }

    pub(crate) fn submit_password(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(prompt) = self.password_prompt.as_mut() else {
            return;
        };
        let value = prompt.input.read(cx).value().to_string();
        if value.trim().is_empty() {
            prompt.error = Some(rust_i18n::t!("prompt.password_required").to_string());
            cx.notify();
            return;
        }
        let Some(mut pending) = self.pending_connect.take() else {
            self.password_prompt = None;
            return;
        };
        if let Connection::Ssh(ssh) = &pending.session.connection
            && let AuthMethod::Password { save: true } = &ssh.auth
        {
            let key = SecretKey::new(pending.session.id, SecretKind::Password);
            if let Err(error) = self.services.secret_store.set(&key, value.as_str()) {
                self.error = Some(localized_error(&error));
            }
        }
        pending.secrets.password = Some(Zeroizing::new(value));
        self.password_prompt = None;
        self.send_connect(
            pending.tab_id,
            pending.connection_id,
            pending.session,
            pending.secrets,
            cx,
        );
    }

    pub(crate) fn cancel_password(&mut self) {
        if let Some(pending) = self.pending_connect.take() {
            self.remove_tab(pending.tab_id);
        }
        self.password_prompt = None;
    }

    pub(crate) fn submit_auth_prompt(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.auth_prompt.take() else {
            return;
        };
        let answers: Vec<String> = state
            .inputs
            .iter()
            .map(|input| input.read(cx).value().to_string())
            .collect();
        let _ = state.responder.send(answers);
        cx.notify();
    }

    pub(crate) fn cancel_auth_prompt(&mut self) {
        self.auth_prompt = None;
    }

    pub(crate) fn render_password_prompt(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let Some(prompt) = self.password_prompt.as_ref() else {
            return div().into_any_element();
        };

        let content = v_flex()
            .gap_3()
            .child(Input::new(&prompt.input).w_full())
            .when_some(prompt.error.clone(), |this, err| {
                this.child(div().text_sm().text_color(p.danger).child(err))
            });

        let footer = h_flex()
            .gap_2()
            .child(
                Button::new("cancel-password")
                    .ghost()
                    .label(rust_i18n::t!("common.cancel").to_string())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.cancel_password();
                        cx.notify();
                    })),
            )
            .child(
                Button::new("submit-password")
                    .primary()
                    .label(rust_i18n::t!("prompt.connect").to_string())
                    .on_click(cx.listener(|this, _, window, cx| this.submit_password(window, cx))),
            );

        let dialog = Self::dialog_frame(
            "password-prompt-modal",
            rust_i18n::t!("prompt.password_title").to_string(),
            Some(rust_i18n::t!("prompt.password_hint").to_string()),
            content,
            Some(footer),
            px(440.0),
            p,
        );

        Self::overlay(dialog, p)
    }

    pub(crate) fn render_auth_prompt(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let Some(prompt) = self.auth_prompt.as_ref() else {
            return div().into_any_element();
        };
        let mut panel = v_flex()
            .id(ElementId::Name("auth-prompt-panel".into()))
            .w(px(480.0))
            .max_h(px(520.0))
            .overflow_y_scroll()
            .gap_3()
            .p_5()
            .rounded_lg()
            .bg(p.popover)
            .text_color(p.popover_foreground)
            .border_1()
            .border_color(p.border)
            .child(
                div()
                    .text_lg()
                    .font_weight(FontWeight::BOLD)
                    .child(rust_i18n::t!("prompt.auth_title").to_string()),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("prompt.auth_hint").to_string()),
            );
        if !prompt.name.is_empty() {
            panel = panel.child(div().text_sm().child(prompt.name.clone()));
        }
        if !prompt.instructions.is_empty() {
            panel = panel.child(
                div()
                    .text_sm()
                    .text_color(p.muted_foreground)
                    .child(prompt.instructions.clone()),
            );
        }
        for (index, prompt_info) in prompt.prompts.iter().enumerate() {
            let input = prompt.inputs[index].clone();
            panel = panel.child(Self::form_field(prompt_info.prompt.clone(), input, p));
        }
        if let Some(error) = &prompt.error {
            panel = panel.child(div().text_sm().text_color(p.danger).child(error.clone()));
        }
        panel = panel.child(
            h_flex()
                .gap_2()
                .justify_end()
                .child(
                    Button::new("cancel-auth")
                        .ghost()
                        .label(rust_i18n::t!("common.cancel").to_string())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.cancel_auth_prompt();
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("submit-auth")
                        .primary()
                        .label(rust_i18n::t!("prompt.submit").to_string())
                        .on_click(cx.listener(|this, _, _, cx| this.submit_auth_prompt(cx))),
                ),
        );
        Self::overlay(panel, p)
    }
}
