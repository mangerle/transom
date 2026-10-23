//! 密钥生成与管理向导的状态、密钥扫描与部署动作。

mod view;

use crate::app::{AppShell, TabStatus};
use crate::palette::localized_error;
use gpui_kit::component::input::InputState;
use gpui_kit::*;
use std::path::PathBuf;
use transom_core::{
    AppCommand, AppError, Connection, ConnectionId, GeneratedKey, KeyAlgorithm, PublicKeyInfo,
    TabId,
};
use zeroize::Zeroizing;

/// 可部署公钥的已连接 SSH 会话。
#[derive(Debug, Clone)]
pub(crate) struct KeyTarget {
    pub(crate) tab_id: TabId,
    pub(crate) connection_id: ConnectionId,
    pub(crate) title: String,
}

pub(crate) struct KeyWizardState {
    ssh_dir: PathBuf,
    pub(crate) algorithm: KeyAlgorithm,
    pub(crate) comment: Entity<InputState>,
    pub(crate) path: Entity<InputState>,
    pub(crate) passphrase: Entity<InputState>,
    pub(crate) keys: Vec<PublicKeyInfo>,
    pub(crate) selected: Option<usize>,
    pub(crate) generating: bool,
    pub(crate) message: Option<String>,
    pub(crate) error: Option<String>,
    _task: Option<Task<()>>,
}

impl KeyWizardState {
    fn new(window: &mut Window, cx: &mut Context<impl Sized>, ssh_dir: PathBuf) -> Self {
        let default_path = ssh_dir.join(KeyAlgorithm::Ed25519.default_file_name());
        let default_path = default_path.display().to_string();
        let comment = default_comment();
        let comment_input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(comment)
                .placeholder("user@host")
        });
        let path_input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(default_path)
                .placeholder("~/.ssh/id_ed25519")
        });
        let passphrase_input = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("留空则不加密私钥")
        });
        Self {
            ssh_dir,
            algorithm: KeyAlgorithm::Ed25519,
            comment: comment_input,
            path: path_input,
            passphrase: passphrase_input,
            keys: Vec::new(),
            selected: None,
            generating: false,
            message: None,
            error: None,
            _task: None,
        }
    }
}

impl AppShell {
    pub(crate) fn open_key_wizard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ssh_dir = default_ssh_dir();
        let mut state = KeyWizardState::new(window, cx, ssh_dir.clone());
        match transom_core::scan_public_keys(&ssh_dir) {
            Ok(keys) => {
                state.selected = (!keys.is_empty()).then_some(0);
                state.keys = keys;
            }
            Err(error) => state.error = Some(localized_error(&error)),
        }
        self.key_wizard = Some(state);
        cx.notify();
    }

    pub(crate) fn close_key_wizard(&mut self, cx: &mut Context<Self>) {
        self.key_wizard = None;
        cx.notify();
    }

    /// 切换算法时，若路径仍是旧算法的默认值则同步为新默认路径。
    pub(crate) fn select_key_algorithm(
        &mut self,
        algorithm: KeyAlgorithm,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.key_wizard.as_mut() else {
            return;
        };
        if state.algorithm == algorithm {
            return;
        }
        let current = state.path.read(cx).value().trim().to_string();
        let old_default = state
            .ssh_dir
            .join(state.algorithm.default_file_name())
            .display()
            .to_string();
        state.algorithm = algorithm;
        if current.is_empty() || current == old_default {
            let new_default = state
                .ssh_dir
                .join(algorithm.default_file_name())
                .display()
                .to_string();
            state
                .path
                .update(cx, |input, cx| input.set_value(&new_default, window, cx));
        }
        state.error = None;
        cx.notify();
    }

    pub(crate) fn generate_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(state) = self.key_wizard.as_mut() else {
            return;
        };
        if state.generating {
            return;
        }
        let path_text = state.path.read(cx).value().trim().to_string();
        if path_text.is_empty() {
            state.error = Some("请填写私钥保存路径".to_string());
            cx.notify();
            return;
        }
        let algorithm = state.algorithm;
        let comment = state.comment.read(cx).value().trim().to_string();
        let passphrase = Zeroizing::new(state.passphrase.read(cx).value().to_string());
        let private_path = expand_home(&path_text);
        let ssh_dir = state.ssh_dir.clone();

        state.generating = true;
        state.error = None;
        state.message = None;
        let task = cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    transom_core::generate_keypair(
                        algorithm,
                        &comment,
                        &private_path,
                        (!passphrase.is_empty()).then_some(passphrase.as_str()),
                    )
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.finish_key_generation(result, ssh_dir, window, cx);
            });
        });
        if let Some(state) = self.key_wizard.as_mut() {
            state._task = Some(task);
        }
        cx.notify();
    }

    fn finish_key_generation(
        &mut self,
        result: Result<GeneratedKey, AppError>,
        ssh_dir: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.key_wizard.as_mut() else {
            return;
        };
        state.generating = false;
        state._task = None;
        match result {
            Ok(generated) => {
                match transom_core::scan_public_keys(&ssh_dir) {
                    Ok(keys) => {
                        state.keys = keys;
                        state.error = None;
                    }
                    Err(error) => state.error = Some(localized_error(&error)),
                }
                state.selected = state
                    .keys
                    .iter()
                    .position(|key| key.path == generated.public_path);
                state.message = Some(format!(
                    "已生成 {} 密钥：{}（{}）",
                    generated.public_key.algorithm,
                    generated.private_path.display(),
                    generated.public_key.fingerprint
                ));
                state
                    .passphrase
                    .update(cx, |input, cx| input.set_value("", window, cx));
            }
            Err(error) => {
                state.error = Some(localized_error(&error));
                state.message = None;
            }
        }
        cx.notify();
    }

    pub(crate) fn select_key(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(state) = self.key_wizard.as_mut() else {
            return;
        };
        if index < state.keys.len() {
            state.selected = Some(index);
            state.error = None;
            cx.notify();
        }
    }

    pub(crate) fn copy_selected_key(&mut self, cx: &mut Context<Self>) {
        let Some(key) = self.selected_key().cloned() else {
            self.set_key_error("请先选择一个公钥", cx);
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(key.openssh));
        self.set_key_notice("公钥已复制到剪贴板", cx);
    }

    pub(crate) fn export_selected_key(&mut self, cx: &mut Context<Self>) {
        let Some(key) = self.selected_key().cloned() else {
            self.set_key_error("请先选择一个公钥", cx);
            return;
        };
        let file_name = key
            .path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| "public_key.pub".to_string());
        let mut dialog = rfd::FileDialog::new()
            .set_title("导出公钥")
            .set_file_name(&file_name)
            .add_filter("OpenSSH 公钥", &["pub"]);
        if let Some(parent) = key.path.parent() {
            dialog = dialog.set_directory(parent);
        }
        let Some(target) = dialog.save_file() else {
            return;
        };
        match std::fs::write(&target, format!("{}\n", key.openssh)) {
            Ok(()) => self.set_key_notice(&format!("公钥已导出到 {}", target.display()), cx),
            Err(error) => self.set_key_error(&format!("导出公钥失败: {error}"), cx),
        }
    }

    pub(crate) fn deploy_selected_key(&mut self, target: KeyTarget, cx: &mut Context<Self>) {
        let Some(key) = self.selected_key().cloned() else {
            self.set_key_error("请先选择一个公钥", cx);
            return;
        };
        let key_name = key
            .path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| "public_key".to_string());
        let command = AppCommand::DeployPublicKey {
            tab_id: target.tab_id,
            connection_id: target.connection_id,
            key_name: key_name.clone(),
            public_key: key.openssh,
        };
        let Some(state) = self.key_wizard.as_mut() else {
            return;
        };
        match self.services.command_tx.send(command) {
            Ok(()) => {
                state.message = Some(format!("正在部署 {key_name} 到 {} ...", target.title));
                state.error = None;
            }
            Err(_) => state.error = Some("发送部署请求失败，应用可能已退出".to_string()),
        }
        cx.notify();
    }

    pub(crate) fn handle_public_key_deployed(
        &mut self,
        key_name: String,
        error: Option<AppError>,
        cx: &mut Context<Self>,
    ) {
        if let Some(state) = self.key_wizard.as_mut() {
            match error {
                Some(error) => {
                    state.error = Some(localized_error(&error));
                    state.message = None;
                }
                None => {
                    state.message = Some(format!("公钥 {key_name} 已写入远端 authorized_keys"));
                    state.error = None;
                }
            }
            cx.notify();
            return;
        }
        match error {
            Some(error) => self.error = Some(localized_error(&error)),
            None => self.status = Some(format!("公钥 {key_name} 已部署")),
        }
        cx.notify();
    }

    fn selected_key(&self) -> Option<&PublicKeyInfo> {
        let state = self.key_wizard.as_ref()?;
        state.selected.and_then(|index| state.keys.get(index))
    }

    fn set_key_notice(&mut self, message: &str, cx: &mut Context<Self>) {
        if let Some(state) = self.key_wizard.as_mut() {
            state.message = Some(message.to_string());
            state.error = None;
        }
        cx.notify();
    }

    fn set_key_error(&mut self, message: &str, cx: &mut Context<Self>) {
        if let Some(state) = self.key_wizard.as_mut() {
            state.error = Some(message.to_string());
            state.message = None;
        }
        cx.notify();
    }

    /// 收集当前已连接、可部署公钥的 SSH 标签页。
    pub(crate) fn key_targets(&self) -> Vec<KeyTarget> {
        self.tabs
            .iter()
            .filter(|tab| {
                matches!(tab.status, TabStatus::Connected)
                    && matches!(tab.session.connection, Connection::Ssh(_))
            })
            .filter_map(|tab| {
                Some(KeyTarget {
                    tab_id: tab.id,
                    connection_id: tab.connection_id?,
                    title: tab.title.clone(),
                })
            })
            .collect()
    }
}

fn default_ssh_dir() -> PathBuf {
    directories::BaseDirs::new()
        .map(|dirs| dirs.home_dir().join(".ssh"))
        .unwrap_or_else(|| PathBuf::from(".ssh"))
}

/// 展开 `~` 与 `~/` 前缀，方便用户按 OpenSSH 习惯输入路径。
fn expand_home(input: &str) -> PathBuf {
    let input = input.trim();
    let Some(dirs) = directories::BaseDirs::new() else {
        return PathBuf::from(input);
    };
    if input == "~" {
        return dirs.home_dir().to_path_buf();
    }
    if let Some(rest) = input.strip_prefix("~/") {
        return dirs.home_dir().join(rest);
    }
    PathBuf::from(input)
}

fn default_comment() -> String {
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "user".to_string());
    let host = std::env::var("HOSTNAME").unwrap_or_else(|_| "transom".to_string());
    format!("{user}@{host}")
}
