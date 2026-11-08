//! 密钥生成与管理向导实体：打开状态、输入控件、公钥清单与全部动作都归它。
//!
//! # 设计原理（Why）
//! 向导此前由 `ModalHub.tools.key_wizard: Option<KeyWizardState>` 加散落的
//! `impl AppShell` 方法维护：打开、扫描、生成、复制、导出与部署分居
//! `keys/mod.rs` 与 `keys/view.rs`，读者要同时读三处才能回答“谁拥有这次打开”。
//! 实体化后，一次打开期的输入控件、公钥清单与在途生成任务同生共死，外壳只
//! 持有句柄；状态变更也只重绘本实体，不再牵动整个外壳。
//!
//! 部署目标列表来自外壳的工作区标签页，实体不反查外壳，由外壳在渲染时以
//! `&[KeyTarget]` 注入；状态栏提示通过 [`KeyWizardEvent`] 上交，弹窗内的
//! 成功/错误文案仍由实体自己持有。
//!
//! # 代价与局限
//! 事件只携带结果，外壳不再能回读实体补数据；“部署中”“已部署”等弹窗内
//! 文案与状态栏文案因此分成两条路径，跨域提示必须在事件里带全。
use super::keygen::{
    GeneratedKey, KeyAlgorithm, PublicKeyInfo, generate_keypair, scan_public_keys,
};
use super::paths::expand_home;
use super::state::{KeyTarget, KeyWizardState};
use crate::app::command_send_error_text;
use crate::palette::localized_error;
use gpui_kit::*;
use std::path::PathBuf;
use transom_core::{AppCommand, AppError, HostCommand};
use zeroize::Zeroizing;

/// 密钥向导交给外壳处理的意图。
///
/// 事件自带全部数据：外壳处理时禁止回读实体，否则会在实体回调期间触发
/// GPUI 的双重租借崩溃。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum KeyWizardEvent {
    /// 状态栏提示（复制、导出成功）。
    Status(String),
    /// 操作失败，由外壳转为应用内错误提示（如命令通道不可用）。
    Error(String),
}

impl EventEmitter<KeyWizardEvent> for KeyWizardView {}

/// 密钥生成与管理向导实体。
pub(crate) struct KeyWizardView {
    /// 未打开时为 `None`；每次打开都重新创建输入控件，避免残留上次的路径与口令。
    state: Option<KeyWizardState>,
    /// SSH 目录在构造时固定，打开与生成完成后都扫描这一份。
    ssh_dir: PathBuf,
    /// 命令通道：部署公钥直接投递，不经过外壳。
    command_tx: flume::Sender<AppCommand>,
    /// 在途生成任务；下一次生成覆盖旧句柄，关闭弹窗时随之取消。
    task: Option<Task<()>>,
}

impl KeyWizardView {
    /// 组装常驻实体；打开状态由 [`Self::open`] 控制。
    pub(crate) fn new(ssh_dir: PathBuf, command_tx: flume::Sender<AppCommand>) -> Self {
        Self {
            state: None,
            ssh_dir,
            command_tx,
            task: None,
        }
    }

    /// 向导是否展开。
    pub(crate) fn is_open(&self) -> bool {
        self.state.is_some()
    }

    /// 当前打开期的只读状态；渲染与状态查询共用同一入口。
    pub(crate) fn state(&self) -> Option<&KeyWizardState> {
        self.state.as_ref()
    }

    /// 打开向导并扫描本地公钥。
    ///
    /// 扫描与迁移前一样在打开时同步执行：目录通常只有几个 `.pub` 文件，
    /// 异步化会让列表先空一帧；真正耗时的密钥生成本身已放到后台线程。
    pub(crate) fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut state = KeyWizardState::new(window, cx, self.ssh_dir.clone());
        match scan_public_keys(&self.ssh_dir) {
            Ok(keys) => {
                state.selected = (!keys.is_empty()).then_some(0);
                state.keys = keys;
            }
            Err(error) => state.error = Some(localized_error(&error)),
        }
        self.state = Some(state);
        cx.notify();
    }

    /// 关闭向导；在途生成任务随句柄一起取消。
    pub(crate) fn close(&mut self, cx: &mut Context<Self>) {
        self.task.take();
        self.state = None;
        cx.notify();
    }

    /// 切换算法时，若路径仍是旧算法的默认值则同步为新默认路径。
    pub(crate) fn select_algorithm(
        &mut self,
        algorithm: KeyAlgorithm,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(wizard) = self.state.as_ref() else {
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
        let path_input = wizard.path.clone();
        let should_update_path = current.is_empty() || current == old_default;
        if let Some(state) = self.state.as_mut() {
            state.algorithm = algorithm;
            state.error = None;
        }
        if should_update_path {
            path_input.update(cx, |input, cx| input.set_value(&new_default, window, cx));
        }
        cx.notify();
    }

    /// 校验路径后在后台生成密钥对，并重新扫描公钥目录。
    ///
    /// 生成与随后的目录扫描都属于阻塞 IO，一并放到后台执行器；任务句柄由
    /// 实体持有，关闭弹窗或重新生成时覆盖旧句柄。
    pub(crate) fn generate_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(wizard) = self.state.as_ref() else {
            return;
        };
        if wizard.generating {
            return;
        }
        let path_text = wizard.path.read(cx).value().trim().to_string();
        if path_text.is_empty() {
            self.set_error(rust_i18n::t!("keys.error_path_empty").as_ref(), cx);
            return;
        }
        let algorithm = wizard.algorithm;
        let comment = wizard.comment.read(cx).value().trim().to_string();
        let passphrase = Zeroizing::new(wizard.passphrase.read(cx).value().to_string());
        let private_path = expand_home(&path_text);
        let ssh_dir = wizard.ssh_dir.clone();
        if let Some(state) = self.state.as_mut() {
            state.generating = true;
            state.error = None;
            state.message = None;
        }
        let task = cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
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
        self.task = Some(task);
        cx.notify();
    }

    /// 后台生成任务收尾：成功后刷新列表并清空口令，失败保留弹窗可就地改正。
    fn finish_key_generation(
        &mut self,
        result: Result<(GeneratedKey, Result<Vec<PublicKeyInfo>, AppError>), AppError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(passphrase_input) = self.state.as_ref().map(|state| state.passphrase.clone())
        else {
            return;
        };
        self.task = None;
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
                if let Some(state) = self.state.as_mut() {
                    state.generating = false;
                    match &scanned {
                        Ok(keys) => {
                            state.keys = keys.clone();
                            state.error = None;
                            state.selected = keys.iter().position(|key| key.path == public_path);
                        }
                        Err(error) => state.error = Some(localized_error(error)),
                    }
                    state.message = Some(message);
                }
                passphrase_input.update(cx, |input, cx| input.set_value("", window, cx));
            }
            Err(error) => {
                if let Some(state) = self.state.as_mut() {
                    state.generating = false;
                    state.error = Some(localized_error(&error));
                    state.message = None;
                }
            }
        }
        cx.notify();
    }

    /// 选中公钥清单里的一项；越界下标被忽略。
    pub(crate) fn select_key(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(state) = self.state.as_mut() else {
            return;
        };
        if index >= state.keys.len() {
            return;
        }
        state.selected = Some(index);
        state.error = None;
        cx.notify();
    }

    /// 复制选中的公钥到剪贴板。
    pub(crate) fn copy_selected_key(&mut self, cx: &mut Context<Self>) {
        let Some(key) = self.selected_key() else {
            self.set_error(rust_i18n::t!("keys.error_select_first").as_ref(), cx);
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(key.openssh));
        let message = rust_i18n::t!("keys.copied").to_string();
        self.set_notice(&message, cx);
        cx.emit(KeyWizardEvent::Status(message));
    }

    /// 导出选中的公钥，文件选择对话框仍在 UI 线程同步弹出。
    pub(crate) fn export_selected_key(&mut self, cx: &mut Context<Self>) {
        let Some(key) = self.selected_key() else {
            self.set_error(rust_i18n::t!("keys.error_select_first").as_ref(), cx);
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
                self.set_notice(&message, cx);
                cx.emit(KeyWizardEvent::Status(message));
            }
            Err(error) => {
                let message =
                    rust_i18n::t!("keys.export_failed", error = error.to_string()).to_string();
                self.set_error(&message, cx);
            }
        }
    }

    /// 把选中的公钥部署到指定的已连接 SSH 会话。
    pub(crate) fn deploy_selected_key(&mut self, target: &KeyTarget, cx: &mut Context<Self>) {
        let Some(key) = self.selected_key() else {
            self.set_error(rust_i18n::t!("keys.error_select_first").as_ref(), cx);
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
        let message = rust_i18n::t!(
            "keys.deploying",
            key = key_name.as_str(),
            host = target.title.as_str()
        )
        .to_string();
        match self.command_tx.try_send(command.into()) {
            Ok(()) => self.set_notice(&message, cx),
            Err(error) => {
                self.set_error(rust_i18n::t!("keys.deploy_send_failed").as_ref(), cx);
                cx.emit(KeyWizardEvent::Error(command_send_error_text(error)));
            }
        }
    }

    /// 部署回执：弹窗内显示结果，状态栏文案由外壳处理。
    pub(crate) fn on_deployed(
        &mut self,
        key_name: String,
        error: Option<AppError>,
        cx: &mut Context<Self>,
    ) {
        if let Some(state) = self.state.as_mut() {
            match &error {
                Some(error) => {
                    state.error = Some(localized_error(error));
                    state.message = None;
                }
                None => {
                    state.message =
                        Some(rust_i18n::t!("keys.deployed", key = key_name.as_str()).to_string());
                    state.error = None;
                }
            }
        }
        cx.notify();
    }

    /// 当前选中的公钥副本。
    fn selected_key(&self) -> Option<PublicKeyInfo> {
        let wizard = self.state.as_ref()?;
        wizard
            .selected
            .and_then(|index| wizard.keys.get(index))
            .cloned()
    }

    /// 在弹窗内显示一条成功提示，并清空错误。
    fn set_notice(&mut self, message: &str, cx: &mut Context<Self>) {
        if let Some(state) = self.state.as_mut() {
            state.message = Some(message.to_string());
            state.error = None;
        }
        cx.notify();
    }

    /// 在弹窗内显示一条错误，并清空成功提示。
    fn set_error(&mut self, message: &str, cx: &mut Context<Self>) {
        if let Some(state) = self.state.as_mut() {
            state.error = Some(message.to_string());
            state.message = None;
        }
        cx.notify();
    }
}
