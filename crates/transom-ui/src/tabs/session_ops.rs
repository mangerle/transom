//! 会话表单操作与钥匙串凭据读写。

use crate::app::AppShell;
use crate::palette::localized_error;
use crate::session_form::SessionForm;
use gpui_kit::*;
use transom_core::{
    AppError, Connection, ErrorCode, SecretKey, SecretKind, SecretStore, Session, SessionId,
};
use zeroize::Zeroizing;

impl AppShell {
    /// 目录里的凭据模板快照，供会话表单渲染可选项。
    pub(crate) fn identity_options(&self) -> Vec<crate::session_form::IdentityOption> {
        self.catalog
            .identities
            .iter()
            .map(|identity| crate::session_form::IdentityOption {
                id: identity.id,
                name: identity.name.clone(),
                username: identity.username.clone(),
            })
            .collect()
    }

    /// 目录里的跳板机候选：只列 SSH 会话，并排除会形成环的候选。
    pub(crate) fn jump_options(
        &self,
        exclude: Option<SessionId>,
    ) -> Vec<crate::session_form::JumpOption> {
        let mut options = Vec::with_capacity(self.catalog.sessions.len());
        for session in &self.catalog.sessions {
            if Some(session.id) == exclude {
                continue;
            }
            let Connection::Ssh(ssh) = &session.connection else {
                continue;
            };
            if let Some(excluded) = exclude
                && self.proxy_chain_reaches(session, excluded)
            {
                continue;
            }
            options.push(crate::session_form::JumpOption {
                id: session.id,
                name: session.name.clone(),
                host: ssh.host.clone(),
                port: ssh.port,
                jump_session_id: ssh.proxy_jump.as_ref().map(|jump| jump.jump_session_id),
            });
        }
        options
    }

    /// 候选会话的跳板链是否会回到 `target`（用于表单阶段的成环预防）。
    fn proxy_chain_reaches(&self, session: &Session, target: SessionId) -> bool {
        let Connection::Ssh(ssh) = &session.connection else {
            return false;
        };
        let mut visited = std::collections::HashSet::with_capacity(4);
        let mut cursor = ssh.proxy_jump.as_ref().map(|jump| jump.jump_session_id);
        while let Some(id) = cursor {
            if id == target {
                return true;
            }
            if !visited.insert(id) {
                return false;
            }
            let Some(next) = self
                .catalog
                .sessions
                .iter()
                .find(|candidate| candidate.id == id)
            else {
                return false;
            };
            let Connection::Ssh(next_ssh) = &next.connection else {
                return false;
            };
            cursor = next_ssh
                .proxy_jump
                .as_ref()
                .map(|jump| jump.jump_session_id);
        }
        false
    }

    /// 获取目录中所有已存在的文件夹路径清单（排重且字典序）。
    pub(crate) fn folder_paths(&self) -> Vec<String> {
        let mut paths: Vec<String> = self
            .catalog
            .folders
            .iter()
            .filter_map(|folder| self.catalog.folder_path(folder.id))
            .collect();
        paths.sort();
        paths.dedup();
        paths
    }

    /// 在会话表单中选择指定分组，并自动应用该分组及其祖先的默认继承属性。
    pub(crate) fn select_form_folder(
        &mut self,
        folder_path: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let defaults = self
            .catalog
            .folders
            .iter()
            .find(|folder| self.catalog.folder_path(folder.id).as_deref() == Some(folder_path))
            .map(|folder| self.catalog.inherited_folder_defaults(folder.id))
            .unwrap_or_default();
        if let Some(form) = self.form.as_mut() {
            form.apply_folder_selection(folder_path, &defaults, window, cx);
        }
    }

    /// 在会话表单中清除分组（选择根层级）。
    pub(crate) fn clear_form_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(form) = self.form.as_mut() {
            form.folder_path
                .update(cx, |state, cx| state.set_value("", window, cx));
            cx.notify();
        }
    }

    pub(crate) fn open_new_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.form = Some(SessionForm::new(
            window,
            cx,
            crate::session_form::SessionFormInit {
                base: None,
                folder_path: None,
                identities: self.identity_options(),
                jump_options: self.jump_options(None),
                folder_options: self.folder_paths(),
                global_proxy: self.settings.proxy.clone(),
                block_view: transom_core::BlockViewOverride::FollowGlobal,
            },
        ));
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
        let block_override = session.block_view;
        let mut form = SessionForm::new(
            window,
            cx,
            crate::session_form::SessionFormInit {
                base: Some(session),
                folder_path,
                identities: self.identity_options(),
                jump_options: self.jump_options(Some(session_id)),
                folder_options: self.folder_paths(),
                global_proxy: self.settings.proxy.clone(),
                block_view: block_override,
            },
        );
        self.prefill_secrets(&mut form, window, cx);
        self.form = Some(form);
        if is_serial {
            self.refresh_serial_ports(window, cx);
        }
    }

    /// 把编辑中会话已保存在钥匙串里的凭据填回表单输入框。
    fn prefill_secrets(
        &mut self,
        form: &mut SessionForm,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(base) = form.base.as_ref() else {
            return;
        };
        let session_id = base.id;
        let mut fields = Vec::new();
        if form.save_password {
            fields.push((SecretKind::Password, form.password.clone()));
        }
        if form.save_passphrase {
            fields.push((SecretKind::KeyPassphrase, form.passphrase.clone()));
        }
        if fields.is_empty() {
            return;
        }
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
                                    .get(&SecretKey::new(session_id, *kind))
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

    /// 保存会话表单：只做目录落盘与凭据写入，不发起连接。
    pub(crate) fn save_session_form(&mut self, cx: &mut Context<Self>) {
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

        let mut secret_writes = Vec::new();
        if built.session.identity_id.is_none() {
            if save_password {
                match &built.password {
                    Some(password) => {
                        secret_writes.push(SecretWrite::Set(password_key, password.clone()))
                    }
                    None => secret_writes.push(SecretWrite::Delete(password_key)),
                }
            } else {
                secret_writes.push(SecretWrite::Delete(password_key));
            }
            if save_passphrase {
                match &built.key_passphrase {
                    Some(passphrase) => {
                        secret_writes.push(SecretWrite::Set(passphrase_key, passphrase.clone()))
                    }
                    None => secret_writes.push(SecretWrite::Delete(passphrase_key)),
                }
            } else {
                secret_writes.push(SecretWrite::Delete(passphrase_key));
            }
        }
        let secret_store = self.services.secret_store.clone();
        cx.spawn(async move |this, cx| {
            let error = if secret_writes.is_empty() {
                None
            } else {
                cx.background_executor()
                    .spawn(
                        async move { apply_secret_writes(secret_store.as_ref(), &secret_writes) },
                    )
                    .await
            };
            let _ = this.update(cx, |this, cx| {
                if let Some(error) = error {
                    this.error = Some(localized_error(&error));
                    cx.notify();
                }
            });
        })
        .detach();

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
}

/// 会话表单保存时需要执行的钥匙串写操作。
enum SecretWrite {
    Set(SecretKey, Zeroizing<String>),
    Delete(SecretKey),
}

/// 顺序执行钥匙串写操作，返回最后一个错误。
fn apply_secret_writes(secret_store: &dyn SecretStore, writes: &[SecretWrite]) -> Option<AppError> {
    let mut last_error = None;
    for write in writes {
        let result = match write {
            SecretWrite::Set(key, value) => secret_store.set(key, value.as_str()),
            SecretWrite::Delete(key) => match secret_store.delete(key) {
                Err(error) if error.code == ErrorCode::SecretStoreUnavailable => Ok(()),
                other => other,
            },
        };
        if let Err(error) = result {
            last_error = Some(error);
        }
    }
    last_error
}
