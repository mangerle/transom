//! 密钥生成与管理向导的状态、密钥扫描与部署动作。

mod keygen;
mod paths;
mod state;
mod view;

pub(crate) use paths::{default_ssh_dir, expand_home};
pub(crate) use state::{KeyTarget, KeyWizardState, KnownHostsModalState};

use crate::app::{AppShell, AppState, TabStatus};
use crate::palette::localized_error;
use gpui_kit::*;
use keygen::{GeneratedKey, KeyAlgorithm, PublicKeyInfo, generate_keypair, scan_public_keys};
use transom_core::{AppError, Connection, HostCommand};
use zeroize::Zeroizing;

impl AppShell {
    /// 当前向导里选中的公钥副本；从设置实体读取。
    fn selected_key(&self, _cx: &App) -> Option<PublicKeyInfo> {
        let wizard = self.state.modals.tools.key_wizard.as_ref()?;
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
        let ssh_dir = default_ssh_dir();
        let mut state = KeyWizardState::new(window, cx, ssh_dir.clone());
        match scan_public_keys(&ssh_dir) {
            Ok(keys) => {
                state.selected = (!keys.is_empty()).then_some(0);
                state.keys = keys;
            }
            Err(error) => state.error = Some(localized_error(&error)),
        }
        self.state.modals.tools.key_wizard = Some(state);
        cx.notify();
    }

    pub(crate) fn close_key_wizard(&mut self, cx: &mut Context<Self>) {
        self.state.modals.tools.key_wizard = None;
        cx.notify();
    }

    /// 切换算法时，若路径仍是旧算法的默认值则同步为新默认路径。
    pub(crate) fn select_key_algorithm(
        &mut self,
        algorithm: KeyAlgorithm,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(wizard) = self.state.modals.tools.key_wizard.clone() else {
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
        if let Some(state) = self.state.modals.tools.key_wizard.as_mut() {
            state.algorithm = algorithm;
            state.error = None;
        }
        if current.is_empty() || current == old_default {
            path_input.update(cx, |input, cx| input.set_value(&new_default, window, cx));
        }
        cx.notify();
    }

    pub(crate) fn generate_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(wizard) = self.state.modals.tools.key_wizard.clone() else {
            return;
        };
        if wizard.generating {
            return;
        }
        let path_text = wizard.path.read(cx).value().trim().to_string();
        if path_text.is_empty() {
            if let Some(state) = self.state.modals.tools.key_wizard.as_mut() {
                state.error = Some(rust_i18n::t!("keys.error_path_empty").to_string());
            }
            cx.notify();
            return;
        }
        let algorithm = wizard.algorithm;
        let comment = wizard.comment.read(cx).value().trim().to_string();
        let passphrase = Zeroizing::new(wizard.passphrase.read(cx).value().to_string());
        let private_path = expand_home(&path_text);
        let ssh_dir = wizard.ssh_dir.clone();
        if let Some(state) = self.state.modals.tools.key_wizard.as_mut() {
            state.generating = true;
            state.error = None;
            state.message = None;
        }
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
        let Some(wizard) = self.state.modals.tools.key_wizard.clone() else {
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
                if let Some(state) = self.state.modals.tools.key_wizard.as_mut() {
                    state.generating = false;
                    match &scanned {
                        Ok(keys) => {
                            state.keys = keys.clone();
                            state.error = None;
                            state.selected = keys
                                .iter()
                                .position(|key| key.path == generated.public_path);
                        }
                        Err(error) => state.error = Some(localized_error(error)),
                    }
                    state.message = Some(message);
                }
                passphrase_input.update(cx, |input, cx| input.set_value("", window, cx));
            }
            Err(error) => {
                if let Some(state) = self.state.modals.tools.key_wizard.as_mut() {
                    state.generating = false;
                    state.error = Some(localized_error(&error));
                    state.message = None;
                }
            }
        }
        cx.notify();
    }

    pub(crate) fn select_key(&mut self, index: usize, cx: &mut Context<Self>) {
        let len = self
            .state
            .modals
            .tools
            .key_wizard
            .as_ref()
            .map(|wizard| wizard.keys.len());
        let Some(len) = len else {
            return;
        };
        if index < len {
            if let Some(state) = self.state.modals.tools.key_wizard.as_mut() {
                state.selected = Some(index);
                state.error = None;
            }
            cx.notify();
        }
    }

    pub(crate) fn copy_selected_key(&mut self, cx: &mut Context<Self>) {
        let Some(key) = self.selected_key(cx) else {
            self.set_key_error(rust_i18n::t!("keys.error_select_first").as_ref(), cx);
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(key.openssh));
        self.set_key_notice(rust_i18n::t!("keys.copied").as_ref(), cx);
    }

    pub(crate) fn export_selected_key(&mut self, cx: &mut Context<Self>) {
        let Some(key) = self.selected_key(cx) else {
            self.set_key_error(rust_i18n::t!("keys.error_select_first").as_ref(), cx);
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
            Ok(()) => self.set_key_notice(
                rust_i18n::t!("keys.exported", path = target.display().to_string()).as_ref(),
                cx,
            ),
            Err(error) => self.set_key_error(
                rust_i18n::t!("keys.export_failed", error = error.to_string()).as_ref(),
                cx,
            ),
        }
    }

    pub(crate) fn deploy_selected_key(&mut self, target: KeyTarget, cx: &mut Context<Self>) {
        let Some(key) = self.selected_key(cx) else {
            self.set_key_error(rust_i18n::t!("keys.error_select_first").as_ref(), cx);
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
        // 先提交命令再改向导状态：send_command 需要可变借用 self
        let sent = self.state.send_command(command);
        let message = rust_i18n::t!(
            "keys.deploying",
            key = key_name.as_str(),
            host = target.title.as_str()
        )
        .to_string();
        if let Some(state) = self.state.modals.tools.key_wizard.as_mut() {
            if sent {
                state.message = Some(message);
                state.error = None;
            } else {
                state.error = Some(rust_i18n::t!("keys.deploy_send_failed").to_string());
            }
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
        if let Some(state) = self.state.modals.tools.key_wizard.as_mut() {
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

    fn set_key_notice(&mut self, message: &str, cx: &mut Context<Self>) {
        if let Some(state) = self.state.modals.tools.key_wizard.as_mut() {
            state.message = Some(message.to_string());
            state.error = None;
        }
        cx.notify();
    }

    fn set_key_error(&mut self, message: &str, cx: &mut Context<Self>) {
        if let Some(state) = self.state.modals.tools.key_wizard.as_mut() {
            state.error = Some(message.to_string());
            state.message = None;
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests;
