//! 密码提示弹窗与主机指纹确认（TOFU）弹窗渲染。

use super::secrets::enable_auth_save;
use crate::app::{AppShell, PendingCredential};
use crate::palette::{Palette, localized_error};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Enter;
use gpui_kit::component::switch::Switch;
use gpui_kit::*;
use transom_core::{Connection, ConnectionId, ErrorCode, PromptKind, SecretKey, SessionId};
use zeroize::Zeroizing;

impl AppShell {
    pub(crate) fn submit_auth_prompt(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.auth_prompt.take() else {
            return;
        };
        let answers: Vec<String> = state
            .inputs
            .iter()
            .map(|input| input.read(cx).value().to_string())
            .collect();
        if let Some(kind) = state.secret_kind
            && state.save
            && let Some(value) = answers.first().filter(|value| !value.is_empty())
        {
            self.pending_credential = Some(PendingCredential {
                session_id: state.session_id,
                owner: self.secret_owner_for(state.session_id),
                connection_id: state.connection_id,
                kind,
                value: Zeroizing::new(value.clone()),
            });
        }
        let _ = state.responder.try_send(answers);
        cx.notify();
    }

    /// 连接成功建立后，把用户勾选保存的凭据写入钥匙串。
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
        let key = SecretKey::new(pending.owner, pending.kind);
        self.mark_credential_saved(pending.owner);
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

    /// 连接失败时丢弃待保存凭据。
    pub(crate) fn discard_pending_credential(&mut self, connection_id: ConnectionId) {
        let matched = self
            .pending_credential
            .as_ref()
            .is_some_and(|pending| pending.connection_id == connection_id);
        if matched {
            self.pending_credential = None;
        }
    }

    /// 会话口令归属：引用凭据模板时归模板。
    fn secret_owner_for(&self, session_id: SessionId) -> transom_core::SecretOwner {
        self.catalog
            .sessions
            .iter()
            .find(|session| session.id == session_id)
            .map_or(transom_core::SecretOwner::Session(session_id), |session| {
                session.secret_owner()
            })
    }

    /// 打开"凭据保存到钥匙串"标记，使目录记录与钥匙串实际状态一致。
    fn mark_credential_saved(&mut self, owner: transom_core::SecretOwner) {
        let flipped = match owner {
            transom_core::SecretOwner::Session(session_id) => self
                .catalog
                .sessions
                .iter_mut()
                .find(|session| session.id == session_id)
                .and_then(|session| match &mut session.connection {
                    Connection::Ssh(ssh) => Some(ssh),
                    _ => None,
                })
                .is_some_and(|ssh| enable_auth_save(&mut ssh.auth)),
            transom_core::SecretOwner::Identity(identity_id) => self
                .catalog
                .identities
                .iter_mut()
                .find(|identity| identity.id == identity_id)
                .is_some_and(|identity| enable_auth_save(&mut identity.auth)),
        };
        if !flipped {
            return;
        }
        self.refresh_tab_sessions();
        self.persist_catalog();
    }

    pub(crate) fn cancel_auth_prompt(&mut self) {
        self.auth_prompt = None;
    }

    /// 用户信任首连主机指纹。
    pub(crate) fn accept_host_key(&mut self, cx: &mut Context<Self>) {
        if let Some(state) = self.host_key_confirm.take() {
            let _ = state.responder.try_send(true);
            cx.notify();
        }
    }

    /// 用户拒绝首连主机指纹。
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
        let client_generated = prompt
            .prompts
            .iter()
            .any(|info| !matches!(info.kind, PromptKind::Text));
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

    /// TOFU 首连主机指纹确认弹窗。
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
