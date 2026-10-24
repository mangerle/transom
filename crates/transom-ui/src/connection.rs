//! 会话连接建立、SSH 配置导入与密码/多因子认证交互模块。

use crate::app::{AppShell, PendingCredential, TabStatus, TerminalTab};
use crate::keymap;
use crate::palette::{Palette, localized_error};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Enter;
use gpui_kit::component::switch::Switch;
use gpui_kit::*;
use transom_core::{
    AppCommand, AppError, AuthMethod, Connection, ConnectionId, ConnectionSecrets, ErrorCode,
    LocalConnection, PromptKind, SecretKey, SecretKind, SecretStore, Session, SessionId, TabId,
    TerminalSize, parse_ssh_config,
};
use zeroize::Zeroizing;

/// 按认证方式收集钥匙串中已保存的连接凭据。
///
/// 阻塞函数（走 D-Bus Secret Service，可能挂起秒级），仅允许在后台执行器
/// 线程池上调用。读取失败不阻断连接：返回错误文本交给界面提示，凭据缺失
/// 时认证阶段会改为向用户索取。
fn collect_connection_secrets(
    secret_store: &dyn SecretStore,
    session: &Session,
) -> (ConnectionSecrets, Option<String>) {
    let mut secrets = ConnectionSecrets::default();
    let mut error = None;
    let Connection::Ssh(ssh) = &session.connection else {
        return (secrets, error);
    };
    match &ssh.auth {
        AuthMethod::Password { .. } => {
            let key = SecretKey::new(session.id, SecretKind::Password);
            match secret_store.get(&key) {
                Ok(Some(secret)) => secrets.password = Some(secret),
                Ok(None) => {}
                // 钥匙串不可用属于可恢复异常：照常发起连接，改为在认证阶段询问用户。
                Err(err) => error = Some(localized_error(&err)),
            }
        }
        AuthMethod::PrivateKey {
            save_passphrase, ..
        } => {
            if *save_passphrase {
                let key = SecretKey::new(session.id, SecretKind::KeyPassphrase);
                match secret_store.get(&key) {
                    Ok(Some(secret)) => secrets.key_passphrase = Some(secret),
                    Ok(None) => {}
                    Err(err) => error = Some(localized_error(&err)),
                }
            }
        }
        AuthMethod::Agent => {}
        _ => {}
    }
    (secrets, error)
}

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

    pub(crate) fn open_local_terminal(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let tab_id = TabId::new();
        let session = Session::new(
            rust_i18n::t!("app.new_local").to_string(),
            Connection::Local(LocalConnection::default()),
        );
        self.create_group(tab_id, session.id);
        self.tabs
            .push(TerminalTab::pending(tab_id, session.clone()));
        self.active_tab = Some(tab_id);
        self.start_connection(tab_id, session, cx);
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
        _window: &mut Window,
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
        self.start_connection(tab_id, session, cx);
    }

    /// 意外断线或连接失败后的自动重连调度。
    ///
    /// 设计原理（Why）：
    /// - 触发条件是"会话显式开启 auto_reconnect 且尝试次数未超上限"，串口与
    ///   本地终端不存在断线重连语义，直接跳过；
    /// - 重试前再次校验标签仍持有失败时的 connection_id：用户手动重连、切换
    ///   会话或关闭标签都会改变它，自动重试随之自然取消，无需独立状态机；
    /// - 退避 2s/5s/10s、上限 3 次：服务器宕机时快速放弃而不是无限打扰；
    ///   连接成功（TerminalReady）后计数清零，再次断线重新完整退避。
    pub(crate) fn schedule_auto_reconnect(
        &mut self,
        tab_id: TabId,
        failed_connection: ConnectionId,
        cx: &mut Context<Self>,
    ) {
        const BACKOFF_SECS: [u64; 3] = [2, 5, 10];
        let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id) else {
            return;
        };
        if tab.connection_id != Some(failed_connection) {
            return;
        }
        let Connection::Ssh(ssh) = &tab.session.connection else {
            return;
        };
        if !ssh.options.auto_reconnect {
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
        let session = tab.session.clone();
        let session_id = tab.session_id;
        let executor = cx.background_executor().clone();
        tab.reconnect_task = Some(cx.spawn(async move |this, cx| {
            executor
                .timer(std::time::Duration::from_secs(delay_secs))
                .await;
            let _ = this.update(cx, |this, cx| {
                let Some(tab) = this.tabs.iter().find(|tab| tab.id == tab_id) else {
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
        if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id) {
            tab.connection_id = Some(connection_id);
            tab.sftp = None;
            tab.forward_statuses.clear();
            tab.network_traffic = None;
            // 旧连接的初始化命令队列随重连一并取消。
            tab._startup_task = None;
        }
        if !matches!(
            session.connection,
            Connection::Ssh(_) | Connection::Local(_) | Connection::Serial(_)
        ) {
            self.error = Some(rust_i18n::t!("error.configuration").to_string());
            return;
        }

        // 凭据只负责"有就带上"：缺失不在这里拦截，认证阶段由 net 层按需向界面索取。
        // 钥匙串走 D-Bus 可能挂起秒级：放到后台线程读取，避免冻结 UI。
        let secret_store = self.services.secret_store.clone();
        let load_session = session.clone();
        cx.spawn(async move |this, cx| {
            let (secrets, error) = cx
                .background_executor()
                .spawn(
                    async move { collect_connection_secrets(secret_store.as_ref(), &load_session) },
                )
                .await;
            let _ = this.update(cx, |this, cx| {
                if let Some(error) = error {
                    this.error = Some(error);
                }
                this.send_connect(tab_id, connection_id, session, secrets, cx);
            });
        })
        .detach();
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
        if !self.send_command(command) {
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
        let _ = state.responder.try_send(answers);
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
        cx: &mut Context<Self>,
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
        // 目录上的保存标记是纯内存更新，立即生效；钥匙串写入走 D-Bus 可能挂起
        // 秒级，放后台执行，避免连接成功的瞬间冻结界面。
        self.mark_credential_saved(pending.session_id);
        let secret_store = self.services.secret_store.clone();
        cx.spawn(async move |this, cx| {
            let error = cx
                .background_executor()
                .spawn(async move {
                    secret_store
                        .set(&key, pending.value.as_str())
                        .err()
                        .filter(|error| error.code != ErrorCode::SecretStoreUnavailable)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if let Some(error) = error {
                    this.error = Some(localized_error(&error));
                }
                cx.notify();
            });
        })
        .detach();
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

    /// 用户信任首连主机指纹：回传确认，known_hosts 记录由认证方完成后继续握手。
    pub(crate) fn accept_host_key(&mut self, cx: &mut Context<Self>) {
        if let Some(state) = self.host_key_confirm.take() {
            let _ = state.responder.try_send(true);
            cx.notify();
        }
    }

    /// 用户拒绝首连主机指纹：连接将以此失败，known_hosts 不落任何记录。
    pub(crate) fn reject_host_key(&mut self, cx: &mut Context<Self>) {
        if let Some(state) = self.host_key_confirm.take() {
            let _ = state.responder.try_send(false);
            cx.notify();
        }
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

    /// TOFU 首连主机指纹确认弹窗：展示主机、密钥类型与 SHA256 指纹，
    /// 用户"信任并连接"才写入 known_hosts，拒绝则中止连接。
    pub(crate) fn render_host_key_confirm(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let Some(state) = self.host_key_confirm.as_ref() else {
            return div().into_any_element();
        };
        let body = rust_i18n::t!(
            "prompt.host_key_body",
            host = state.host.as_str(),
            port = state.port
        )
        .to_string();
        let content = v_flex()
            .gap_3()
            .child(div().text_sm().child(body))
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child(format!("{}: {}", state.key_type, state.host)),
                    )
                    // 指纹需要逐字符核对，用主题等宽字体避免视觉混淆
                    .child(
                        div()
                            .text_sm()
                            .font_family(p.mono_font_family.clone())
                            .child(state.fingerprint.clone()),
                    ),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(p.danger)
                    .child(rust_i18n::t!("prompt.host_key_warning").to_string()),
            );
        let footer = h_flex()
            .gap_2()
            .justify_end()
            .child(
                Button::new("reject-host-key")
                    .ghost()
                    .label(rust_i18n::t!("prompt.host_key_reject").to_string())
                    .on_click(cx.listener(|this, _, _, cx| this.reject_host_key(cx))),
            )
            .child(
                Button::new("accept-host-key")
                    .primary()
                    .label(rust_i18n::t!("prompt.host_key_trust").to_string())
                    .on_click(cx.listener(|this, _, _, cx| this.accept_host_key(cx))),
            );
        let dialog = Self::dialog_frame(
            "host-key-modal",
            rust_i18n::t!("prompt.host_key_title").to_string(),
            Some(rust_i18n::t!("prompt.host_key_hint").to_string()),
            content,
            Some(footer),
            px(440.0),
            p,
        );
        Self::overlay(dialog, p)
    }
}
