//! 会话连接建立、SSH 配置导入与密码/多因子认证交互模块。

use crate::app::{AppShell, PendingCredential, TerminalTab};
use crate::keymap;
use crate::palette::{Palette, localized_error};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Enter;
use gpui_kit::component::switch::Switch;
use gpui_kit::*;
use transom_core::{
    AppCommand, AuthMethod, Connection, ConnectionId, ConnectionSecrets, ErrorCode,
    LocalConnection, PromptKind, SecretKey, SecretKind, Session, SessionId, TabId, TerminalSize,
    parse_ssh_config,
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
        self.create_group(tab_id, session.id);
        self.tabs
            .push(TerminalTab::pending(tab_id, session.clone()));
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

        let folder = self.catalog.ensure_folder_path("SSH Config");
        let mut added_count = 0;
        for mut session in parsed {
            if !self.catalog.sessions.iter().any(|s| s.name == session.name) {
                session.folder_id = folder;
                self.catalog.sessions.push(session);
                added_count += 1;
            }
        }

        if added_count > 0 {
            if self.persist_catalog() {
                self.status =
                    Some(rust_i18n::t!("app.import_success", count = added_count).to_string());
            }
        } else {
            self.status = Some(rust_i18n::t!("app.import_none").to_string());
        }
        cx.notify();
    }

    /// 为某个会话新建一个标签组并发起连接。
    ///
    /// 不做"该会话已开就切过去"的去重：一个标签可以有多个同会话窗格，标签本身
    /// 也允许并列多个同会话实例（不同目录、不同任务），去重会让"再开一个"这个
    /// 明确动作没有入口。
    pub(crate) fn connect_session(
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

        let tab_id = TabId::new();
        self.create_group(tab_id, session_id);
        self.tabs
            .push(TerminalTab::pending(tab_id, session.clone()));
        self.active_tab = Some(tab_id);
        self.start_connection(tab_id, session, window, cx);
    }

    pub(crate) fn start_connection(
        &mut self,
        tab_id: TabId,
        session: Session,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let connection_id = ConnectionId::new();
        if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id) {
            tab.connection_id = Some(connection_id);
            tab.sftp = None;
            tab.forward_statuses.clear();
            tab.network_traffic = None;
            // 旧连接的初始化命令队列随重连一并取消。
            tab._startup_task = None;
        }
        // 凭据只负责"有就带上"：缺失不在这里拦截，认证阶段由 net 层按需向界面索取。
        let mut secrets = ConnectionSecrets::default();

        match &session.connection {
            Connection::Ssh(ssh) => match &ssh.auth {
                AuthMethod::Password { .. } => {
                    let key = SecretKey::new(session.id, SecretKind::Password);
                    match self.services.secret_store.get(&key) {
                        Ok(Some(secret)) => secrets.password = Some(secret),
                        Ok(None) => {}
                        // 钥匙串不可用属于可恢复异常：照常发起连接，改为在认证阶段询问用户。
                        Err(error) => self.error = Some(localized_error(&error)),
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
            Connection::Local(_) | Connection::Serial(_) => {}
            _ => {
                self.error = Some(rust_i18n::t!("error.configuration").to_string());
                return;
            }
        }

        self.send_connect(tab_id, connection_id, session, secrets, cx);
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

    pub(crate) fn submit_auth_prompt(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.auth_prompt.take() else {
            return;
        };
        let answers: Vec<String> = state
            .inputs
            .iter()
            .map(|input| input.read(cx).value().to_string())
            .collect();
        // 勾选了保存只先暂存：此刻还不知道服务器是否接受这个密码，写早了会把错口令
        // 持久化，见 `PendingCredential`。连接成功建立时才由 `flush_pending_credential` 落盘。
        if let Some(kind) = state.secret_kind
            && state.save
            && let Some(value) = answers.first().filter(|value| !value.is_empty())
        {
            self.pending_credential = Some(PendingCredential {
                session_id: state.session_id,
                connection_id: state.connection_id,
                kind,
                value: Zeroizing::new(value.clone()),
            });
        }
        let _ = state.responder.send(answers);
        cx.notify();
    }

    /// 连接成功建立后，把用户勾选保存的凭据写入钥匙串。
    ///
    /// 同时打开会话记录上的保存标记：否则下次在会话表单里保存，会因为标记仍是
    /// "不保存"而把这条凭据删掉，用户又被问一遍。
    pub(crate) fn flush_pending_credential(
        &mut self,
        session_id: SessionId,
        connection_id: ConnectionId,
    ) {
        let matched = self.pending_credential.as_ref().is_some_and(|pending| {
            pending.session_id == session_id && pending.connection_id == connection_id
        });
        if !matched {
            return;
        }
        let Some(pending) = self.pending_credential.take() else {
            return;
        };
        let key = SecretKey::new(pending.session_id, pending.kind);
        if let Err(error) = self.services.secret_store.set(&key, &pending.value)
            && error.code != ErrorCode::SecretStoreUnavailable
        {
            self.error = Some(localized_error(&error));
        }
        self.mark_credential_saved(pending.session_id);
    }

    /// 连接失败时丢弃待保存凭据，避免这次没通过的口令被下一条连接写入。
    pub(crate) fn discard_pending_credential(&mut self, connection_id: ConnectionId) {
        let matched = self
            .pending_credential
            .as_ref()
            .is_some_and(|pending| pending.connection_id == connection_id);
        if matched {
            self.pending_credential = None;
        }
    }

    /// 打开会话上的"凭据保存到钥匙串"标记，使目录记录与钥匙串实际状态一致。
    fn mark_credential_saved(&mut self, session_id: SessionId) {
        let Some(session) = self
            .catalog
            .sessions
            .iter_mut()
            .find(|session| session.id == session_id)
        else {
            return;
        };
        let Connection::Ssh(ssh) = &mut session.connection else {
            return;
        };
        match &mut ssh.auth {
            AuthMethod::Password { save } => *save = true,
            AuthMethod::PrivateKey {
                save_passphrase, ..
            } => *save_passphrase = true,
            // agent 等方式没有可保存的凭据，能到这里说明目录已被改过，保持原样即可。
            _ => {}
        }
        self.refresh_tab_sessions();
        self.persist_catalog();
    }

    pub(crate) fn cancel_auth_prompt(&mut self) {
        self.auth_prompt = None;
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
        // 我们自己生成的提示（登录密码、私钥口令）没有服务器文本可依，只能按语义类别
        // 取本地化标签；服务器下发的 keyboard-interactive 文本保持原样，避免改写用户
        // 需要核对的挑战内容。
        let client_generated = prompt
            .prompts
            .iter()
            .any(|info| !matches!(info.kind, PromptKind::Text));
        // 服务器可能下发多条挑战，内容区自带有限高度的滚动区。
        // 回车与"提交"按钮等效：这个弹窗里没有需要换行的输入，Enter 不会被文本域吃掉。
        let mut content = v_flex()
            .id(ElementId::Name("auth-prompt-body".into()))
            .max_h(px(360.0))
            .overflow_y_scroll()
            .gap_3()
            .on_action(cx.listener(|this, _: &Enter, _, cx| {
                this.submit_auth_prompt(cx);
            }));
        if !prompt.name.is_empty() {
            content = content.child(div().text_sm().child(prompt.name.clone()));
        }
        if !prompt.instructions.is_empty() {
            content = content.child(
                div()
                    .text_sm()
                    .text_color(p.muted_foreground)
                    .child(prompt.instructions.clone()),
            );
        }
        for (index, prompt_info) in prompt.prompts.iter().enumerate() {
            let input = prompt.inputs[index].clone();
            let label = match prompt_info.kind {
                PromptKind::Password => rust_i18n::t!("prompt.password_field").to_string(),
                PromptKind::Passphrase => rust_i18n::t!("prompt.passphrase_field").to_string(),
                PromptKind::Text => prompt_info.prompt.clone(),
            };
            let field = if prompt_info.echo {
                Self::form_field(label, input, p)
            } else {
                Self::secret_field(label, input, p)
            };
            content = content.child(field);
        }

        if prompt.secret_kind.is_some() {
            content = content.child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .text_sm()
                            .child(rust_i18n::t!("prompt.save_credential").to_string()),
                    )
                    .child(
                        Switch::new("toggle-save-credential")
                            .small()
                            .checked(prompt.save)
                            .on_click(cx.listener(|this, &checked, _, cx| {
                                if let Some(state) = this.auth_prompt.as_mut() {
                                    state.save = checked;
                                }
                                cx.notify();
                            })),
                    ),
            );
        }

        // 错误提示与按钮一起放在外壳上：内容区会滚动，报错不能被滚出视野。
        let mut footer = v_flex().gap_2().items_end();
        if let Some(error) = &prompt.error {
            footer = footer.child(div().text_sm().text_color(p.danger).child(error.clone()));
        }
        footer = footer.child(
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
        let hint = if client_generated {
            rust_i18n::t!("prompt.credential_hint").to_string()
        } else {
            rust_i18n::t!("prompt.auth_hint").to_string()
        };
        let dialog = Self::dialog_frame(
            "auth-prompt-modal",
            rust_i18n::t!("prompt.auth_title").to_string(),
            Some(hint),
            content,
            Some(footer),
            px(440.0),
            p,
        );
        Self::overlay(dialog, p)
    }
}
