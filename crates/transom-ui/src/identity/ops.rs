//! 凭据模板的异步钥匙串加载、保存落盘与删除处理。

use super::types::{IdentityEditState, IdentityForm, build_identity};
use crate::app::AppShell;
use crate::palette::localized_error;
use crate::session_form::{AuthChoice, text_input};
use gpui_kit::{Context, Window};
use transom_core::{
    AppError, AuthMethod, ErrorCode, IdentityId, SecretKey, SecretKind, SecretOwner,
};

impl AppShell {
    /// 打开模板编辑器；编辑时后台读取已保存口令回填输入框。
    pub(crate) fn open_identity_editor(
        &mut self,
        id: Option<IdentityId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let existing = id.and_then(|id| self.catalog.identity(id).cloned());
        let (auth, key_path) = match existing.as_ref().map(|identity| &identity.auth) {
            Some(AuthMethod::PrivateKey { path, .. }) => {
                (AuthChoice::PrivateKey, path.display().to_string())
            }
            Some(AuthMethod::Password { .. }) => (AuthChoice::Password, String::new()),
            _ => (AuthChoice::Agent, String::new()),
        };
        let save_password = matches!(
            existing.as_ref().map(|identity| &identity.auth),
            Some(AuthMethod::Password { save: true })
        );
        let save_passphrase = matches!(
            existing.as_ref().map(|identity| &identity.auth),
            Some(AuthMethod::PrivateKey {
                save_passphrase: true,
                ..
            })
        );
        let owner = existing
            .as_ref()
            .map(|identity| SecretOwner::Identity(identity.id));
        let state = IdentityEditState {
            base: existing.as_ref().map(|identity| identity.id),
            name: text_input(
                window,
                cx,
                &existing
                    .as_ref()
                    .map(|i| i.name.clone())
                    .unwrap_or_default(),
                "",
            ),
            username: text_input(
                window,
                cx,
                &existing
                    .as_ref()
                    .map(|i| i.username.clone())
                    .unwrap_or_default(),
                "",
            ),
            auth,
            key_path: text_input(window, cx, &key_path, ""),
            password: text_input(
                window,
                cx,
                "",
                &rust_i18n::t!("identity.password_placeholder"),
            ),
            passphrase: text_input(
                window,
                cx,
                "",
                &rust_i18n::t!("identity.passphrase_placeholder"),
            ),
            save_password,
            save_passphrase,
            error: None,
        };
        self.settings_modal.identity_edit = Some(state);
        if let Some(owner) = owner {
            self.prefill_identity_secrets(owner, window, cx);
        }
        cx.notify();
    }

    /// 读取模板已存口令并回填；钥匙串走 D-Bus 可能挂起秒级，放后台执行。
    fn prefill_identity_secrets(
        &mut self,
        owner: SecretOwner,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(edit) = self.settings_modal.identity_edit.as_ref() else {
            return;
        };
        let fields = [
            (SecretKind::Password, edit.password.clone()),
            (SecretKind::KeyPassphrase, edit.passphrase.clone()),
        ];
        let secret_store = self.services.secret_store.clone();
        let executor = cx.background_executor().clone();
        let read_fields = fields.clone();
        window
            .spawn(cx, async move |cx| {
                let secrets = executor
                    .spawn(async move {
                        read_fields
                            .iter()
                            .map(|(kind, _)| {
                                secret_store
                                    .get(&SecretKey::new(owner, *kind))
                                    .ok()
                                    .flatten()
                            })
                            .collect::<Vec<_>>()
                    })
                    .await;
                let _ = cx.update(|window, cx| {
                    for ((_, field), secret) in fields.into_iter().zip(secrets) {
                        if let Some(secret) = secret {
                            field.update(cx, |state, cx| {
                                state.set_value(secret.as_str(), window, cx);
                            });
                        }
                    }
                });
            })
            .detach();
    }

    pub(crate) fn cancel_identity_edit(&mut self, cx: &mut Context<Self>) {
        self.settings_modal.identity_edit = None;
        cx.notify();
    }

    /// 保存模板：写目录并后台落口令。
    pub(crate) fn save_identity(&mut self, cx: &mut Context<Self>) {
        let Some(edit) = self.settings_modal.identity_edit.as_ref() else {
            return;
        };
        let name = edit.name.read(cx).value().to_string();
        let username = edit.username.read(cx).value().to_string();
        let key_path = edit.key_path.read(cx).value().to_string();
        let password = edit.password.read(cx).value().to_string();
        let passphrase = edit.passphrase.read(cx).value().to_string();
        let values = IdentityForm {
            name: &name,
            username: &username,
            auth: edit.auth,
            key_path: &key_path,
            save_password: edit.save_password,
            save_passphrase: edit.save_passphrase,
            password: &password,
            passphrase: &passphrase,
            base: edit.base,
        };
        let built = build_identity(&values);
        let built = match built {
            Ok(built) => built,
            Err(error) => {
                if let Some(edit) = self.settings_modal.identity_edit.as_mut() {
                    edit.error = Some(error);
                }
                cx.notify();
                return;
            }
        };

        let identity = built.identity;
        let replaced = match self
            .catalog
            .identities
            .iter_mut()
            .find(|existing| existing.id == identity.id)
        {
            Some(existing) => {
                *existing = identity.clone();
                true
            }
            None => false,
        };
        if !replaced {
            self.catalog.identities.push(identity);
        }
        self.settings_modal.identity_edit = None;
        self.refresh_tab_sessions();
        self.persist_catalog();
        self.status = Some(
            rust_i18n::t!(if replaced {
                "identity.updated"
            } else {
                "identity.created"
            })
            .to_string(),
        );

        let writes = built.writes;
        if !writes.is_empty() {
            let secret_store = self.services.secret_store.clone();
            cx.spawn(async move |this, cx| {
                let error = cx
                    .background_executor()
                    .spawn(async move {
                        let mut last = None;
                        for (key, value) in &writes {
                            let result = match value {
                                Some(value) => secret_store.set(key, value.as_str()),
                                None => secret_store.delete(key),
                            };
                            if let Err(error) = result {
                                last = Some(error);
                            }
                        }
                        last
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
        cx.notify();
    }

    /// 删除模板；仍被会话引用时拒绝并说明引用数，成功后清理其口令槽位。
    pub(crate) fn delete_identity(&mut self, id: IdentityId, cx: &mut Context<Self>) {
        let name = self
            .catalog
            .identity(id)
            .map(|identity| identity.name.clone())
            .unwrap_or_default();
        if let Err(error) = self.catalog.delete_identity(id) {
            self.error = Some(
                AppError::with_detail(ErrorCode::Configuration, error.to_string()).to_string(),
            );
            cx.notify();
            return;
        }
        if self
            .settings_modal
            .identity_edit
            .as_ref()
            .is_some_and(|edit| edit.base == Some(id))
        {
            self.settings_modal.identity_edit = None;
        }
        self.status = Some(rust_i18n::t!("identity.deleted", name = name).to_string());
        self.persist_catalog();

        let secret_store = self.services.secret_store.clone();
        cx.spawn(async move |this, cx| {
            let error = cx
                .background_executor()
                .spawn(async move {
                    let mut last = None;
                    for kind in [SecretKind::Password, SecretKind::KeyPassphrase] {
                        if let Err(error) =
                            secret_store.delete(&SecretKey::new(SecretOwner::Identity(id), kind))
                        {
                            last = Some(error);
                        }
                    }
                    last
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
        cx.notify();
    }
}
