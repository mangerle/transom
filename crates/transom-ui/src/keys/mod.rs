//! 密钥生成与管理向导的状态、密钥扫描与部署动作。

mod entity;
mod keygen;
mod known_hosts_view;
mod paths;
mod state;
mod view;

pub(crate) use entity::KeyWizardView;
pub(crate) use known_hosts_view::{KnownHostsEvent, KnownHostsView};
pub(crate) use paths::{default_ssh_dir, expand_home};
pub(crate) use state::{KeyTarget, KeyWizardState};

use crate::app::{AppShell, AppState, TabStatus};
use crate::palette::localized_error;
use gpui_kit::*;
use keygen::{GeneratedKey, KeyAlgorithm, PublicKeyInfo, generate_keypair, scan_public_keys};
use transom_core::{AppError, Connection, HostCommand};
use zeroize::Zeroizing;

impl AppShell {
    /// 当前向导里选中的公钥副本；从实体读取。
    fn selected_key(&self, cx: &App) -> Option<PublicKeyInfo> {
        let view = self.state.modals.tools.key_wizard.read(cx);
        let wizard = view.state()?;
        wizard
            .selected
            .and_then(|index| wizard.keys.get(index))
            .cloned()
    }
}

impl AppState {
    pub(crate) fn key_targets(&self) -> Vec<KeyTarget> {
        self.workspace
            .tabs
            .iter()
            .filter(|tab| {
                matches!(tab.status, TabStatus::Connected)
                    && matches!(tab.session.connection, Connection::Ssh(_))
            })
            .filter_map(|tab| {
                Some(KeyTarget {
                    tab_id: tab.id,
                    connection_id: tab.connection_id?,
                    title: tab.display_name(),
                })
            })
            .collect()
    }
}

impl AppShell {
    pub(crate) fn open_key_wizard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let view = self.state.modals.tools.key_wizard.clone();
        view.update(cx, |view, cx| view.open(window, cx));
        cx.notify();
    }

    pub(crate) fn close_key_wizard(&mut self, cx: &mut Context<Self>) {
        let view = self.state.modals.tools.key_wizard.clone();
        view.update(cx, |view, cx| view.close(cx));
        cx.notify();
    }

    /// 切换算法时，若路径仍是旧算法的默认值则同步为新默认路径。
    pub(crate) fn select_key_algorithm(
        &mut self,
        algorithm: KeyAlgorithm,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = self.state.modals.tools.key_wizard.clone();
        let Some(wizard) = view.read(cx).state().cloned() else {
            return;
        };
        if wizard.algorithm == algorithm {
            return;
        }
        let current = wizard.path.read(cx).value().trim().to_string();
        let old_default = wizard
            .ssh_dir
            .join(wizard.algorithm.default_file_name())
            .display()
            .to_string();
        let new_default = wizard
            .ssh_dir
            .join(algorithm.default_file_name())
            .display()
            .to_string();
        let path_input = wizard.path;
        view.update(cx, |view, cx| {
            if let Some(state) = view.state_mut() {
                state.algorithm = algorithm;
                state.error = None;
            }
            cx.notify();
        });
        if current.is_empty() || current == old_default {
            path_input.update(cx, |input, cx| input.set_value(&new_default, window, cx));
        }
        cx.notify();
    }

    pub(crate) fn generate_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let view = self.state.modals.tools.key_wizard.clone();
        let Some(wizard) = view.read(cx).state().cloned() else {
            return;
        };
        if wizard.generating {
            return;
        }
        let path_text = wizard.path.read(cx).value().trim().to_string();
        if path_text.is_empty() {
            view.update(cx, |view, cx| {
                view.set_error(rust_i18n::t!("keys.error_path_empty").as_ref(), cx);
            });
            return;
        }
        let algorithm = wizard.algorithm;
        let comment = wizard.comment.read(cx).value().trim().to_string();
        let passphrase = Zeroizing::new(wizard.passphrase.read(cx).value().to_string());
        let private_path = expand_home(&path_text);
        let ssh_dir = wizard.ssh_dir.clone();
        view.update(cx, |view, cx| {
            if let Some(state) = view.state_mut() {
                state.generating = true;
                state.error = None;
                state.message = None;
            }
            cx.notify();
        });
        let task = cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    // 生成与随后的公钥目录扫描都属于阻塞 IO：一并放到后台线程，
                    // 避免生成完成后在 UI 线程重新扫描整个 ssh 目录。
                    let generated = generate_keypair(
                        algorithm,
                        &comment,
                        &private_path,
                        (!passphrase.is_empty()).then_some(passphrase.as_str()),
                    )?;
                    Ok((generated, scan_public_keys(&ssh_dir)))
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.finish_key_generation(result, window, cx);
            });
        });
        self.state.tasks.background.keygen = Some(task);
        cx.notify();
    }

    fn finish_key_generation(
        &mut self,
        result: Result<(GeneratedKey, Result<Vec<PublicKeyInfo>, AppError>), AppError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = self.state.modals.tools.key_wizard.clone();
        let Some(wizard) = view.read(cx).state().cloned() else {
            return;
        };
        self.state.tasks.background.keygen = None;
        let passphrase_input = wizard.passphrase;
        match result {
            Ok((generated, scanned)) => {
                let message = rust_i18n::t!(
                    "keys.generated",
                    algorithm = generated.public_key.algorithm.as_str(),
                    path = generated.private_path.display().to_string(),
                    fingerprint = generated.public_key.fingerprint.as_str()
                )
                .to_string();
                let public_path = generated.public_path;
                view.update(cx, |view, cx| {
                    if let Some(state) = view.state_mut() {
                        state.generating = false;
                        match &scanned {
                            Ok(keys) => {
                                state.keys = keys.clone();
                                state.error = None;
                                state.selected =
                                    keys.iter().position(|key| key.path == public_path);
                            }
                            Err(error) => state.error = Some(localized_error(error)),
                        }
                        state.message = Some(message);
                    }
                    cx.notify();
                });
                passphrase_input.update(cx, |input, cx| input.set_value("", window, cx));
            }
            Err(error) => {
                let error = localized_error(&error);
                view.update(cx, |view, cx| {
                    if let Some(state) = view.state_mut() {
                        state.generating = false;
                        state.error = Some(error);
                        state.message = None;
                    }
                    cx.notify();
                });
            }
        }
        cx.notify();
    }

    pub(crate) fn select_key(&mut self, index: usize, cx: &mut Context<Self>) {
        let view = self.state.modals.tools.key_wizard.clone();
        let Some(len) = view.read(cx).state().map(|wizard| wizard.keys.len()) else {
            return;
        };
        if index < len {
            view.update(cx, |view, cx| {
                if let Some(state) = view.state_mut() {
                    state.selected = Some(index);
                    state.error = None;
                }
                cx.notify();
            });
            cx.notify();
        }
    }

    pub(crate) fn copy_selected_key(&mut self, cx: &mut Context<Self>) {
        let view = self.state.modals.tools.key_wizard.clone();
        let Some(key) = self.selected_key(cx) else {
            view.update(cx, |view, cx| {
                view.set_error(rust_i18n::t!("keys.error_select_first").as_ref(), cx);
            });
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(key.openssh));
        view.update(cx, |view, cx| {
            view.set_notice(rust_i18n::t!("keys.copied").as_ref(), cx);
        });
        cx.notify();
    }

    pub(crate) fn export_selected_key(&mut self, cx: &mut Context<Self>) {
        let view = self.state.modals.tools.key_wizard.clone();
        let Some(key) = self.selected_key(cx) else {
            view.update(cx, |view, cx| {
                view.set_error(rust_i18n::t!("keys.error_select_first").as_ref(), cx);
            });
            return;
        };
        let file_name = key
            .path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| "public_key.pub".to_string());
        let mut dialog = rfd::FileDialog::new()
            .set_title(rust_i18n::t!("keys.export_title").to_string())
            .set_file_name(&file_name)
            .add_filter(rust_i18n::t!("keys.openssh_filter").to_string(), &["pub"]);
        if let Some(parent) = key.path.parent() {
            dialog = dialog.set_directory(parent);
        }
        let Some(target) = dialog.save_file() else {
            return;
        };
        match std::fs::write(&target, format!("{}\n", key.openssh)) {
            Ok(()) => {
                let message =
                    rust_i18n::t!("keys.exported", path = target.display().to_string()).to_string();
                view.update(cx, |view, cx| view.set_notice(&message, cx));
            }
            Err(error) => {
                let message =
                    rust_i18n::t!("keys.export_failed", error = error.to_string()).to_string();
                view.update(cx, |view, cx| view.set_error(&message, cx));
            }
        }
        cx.notify();
    }

    pub(crate) fn deploy_selected_key(&mut self, target: KeyTarget, cx: &mut Context<Self>) {
        let view = self.state.modals.tools.key_wizard.clone();
        let Some(key) = self.selected_key(cx) else {
            view.update(cx, |view, cx| {
                view.set_error(rust_i18n::t!("keys.error_select_first").as_ref(), cx);
            });
            return;
        };
        let key_name = key
            .path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| "public_key".to_string());
        let command = HostCommand::DeployPublicKey {
            tab_id: target.tab_id,
            connection_id: target.connection_id,
            key_name: key_name.clone(),
            public_key: key.openssh,
        };
        let send_error = view.read(cx).command_tx().try_send(command.into()).err();
        let message = rust_i18n::t!(
            "keys.deploying",
            key = key_name.as_str(),
            host = target.title.as_str()
        )
        .to_string();
        view.update(cx, |view, cx| {
            if send_error.is_none() {
                view.set_notice(&message, cx);
            } else {
                view.set_error(rust_i18n::t!("keys.deploy_send_failed").as_ref(), cx);
            }
        });
        if let Some(error) = send_error {
            self.state
                .set_error(crate::app::command_send_error_text(error));
        }
        cx.notify();
    }

    pub(crate) fn handle_public_key_deployed(
        &mut self,
        key_name: String,
        error: Option<AppError>,
        cx: &mut Context<Self>,
    ) {
        let key = key_name.clone();
        let view = self.state.modals.tools.key_wizard.clone();
        view.update(cx, |view, cx| {
            if let Some(state) = view.state_mut() {
                match &error {
                    Some(error) => {
                        state.error = Some(localized_error(error));
                        state.message = None;
                    }
                    None => {
                        state.message =
                            Some(rust_i18n::t!("keys.deployed", key = key.as_str()).to_string());
                        state.error = None;
                    }
                }
            }
            cx.notify();
        });
        match error {
            Some(error) => self.state.set_error(localized_error(&error)),
            None => {
                self.state.status = Some(
                    rust_i18n::t!("keys.deployed_status", key = key_name.as_str()).to_string(),
                );
            }
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests;
