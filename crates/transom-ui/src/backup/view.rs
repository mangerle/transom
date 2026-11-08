//! 备份导入/导出弹窗实体：状态、文件选择、后台任务与事件上报都归它。
//!
//! # 设计原理（Why）
//! 备份弹窗此前是外壳字段加散落方法：打开、校验、文件对话框、后台任务与收尾
//! 分居 `backup/prompt.rs` 与 `backup/mod.rs`，读者必须同时读两处才能回答
//! “谁拥有这次提交”。实体化后，弹窗状态与在途任务同生共死，外壳只持有句柄；
//! 目录合并、持久化与状态栏提示仍属外壳职责，通过 [`BackupPromptEvent`] 上交。
//!
//! 打开时注入目录快照：导出必须写用户点击“导出”那一刻的目录，后台任务运行
//! 期间其他入口修改目录不应混入本次备份；导入同理，合并基线需与打开时一致。
//!
//! # 代价与局限
//! 事件只携带结果，外壳不再能通过读实体补数据；所有跨域信息必须在事件里带全。
//! 文件对话框仍在 UI 线程同步弹出（与迁移前一致），这是 `rfd` 的平台约束。
use super::helpers::ensure_backup_extension;
use super::prompt::{BackupAction, BackupPromptState};
use crate::palette::localized_error;
use gpui_kit::component::input::InputState;
use gpui_kit::*;
use std::path::PathBuf;
use std::sync::Arc;
use transom_core::{
    AppError, BackupExportOptions, BackupExportReport, BackupImportOptions, BackupService,
    ErrorCode, SecretRestoreReport, SessionCatalog,
};
use zeroize::Zeroizing;

/// 备份实体交给外壳处理的意图。
///
/// 事件自带全部数据：外壳处理时禁止回读实体，否则会在实体回调期间触发
/// GPUI 的双重租借崩溃。
#[derive(Debug)]
pub(crate) enum BackupPromptEvent {
    /// 导出完成；状态栏文案与“包含口令”补充由外壳组装。
    Exported {
        path: PathBuf,
        report: BackupExportReport,
    },
    /// 导入完成：目录快照交由外壳合并，口令写回结果单独汇报。
    Imported {
        catalog: SessionCatalog,
        /// 口令写回结果；钥匙串不可用时携带错误，会话导入本身不受影响。
        secrets: Result<SecretRestoreReport, AppError>,
    },
    /// 后台任务失败；错误已写入弹窗，外壳只记录日志、不再重复提示。
    Error { error: AppError },
}

/// 后台备份任务的结果。
enum BackupOutcome {
    Exported {
        path: PathBuf,
        report: BackupExportReport,
    },
    Imported {
        catalog: SessionCatalog,
        secrets: Result<SecretRestoreReport, AppError>,
    },
}

/// 备份后台任务输入：方向、服务、路径与口令。
struct BackupJob {
    is_export: bool,
    backup: Arc<dyn BackupService>,
    path: PathBuf,
    catalog: SessionCatalog,
    passphrase: Option<Zeroizing<String>>,
    include_secrets: bool,
}

/// 提交前的一次性输入快照；把“读控件”与“校验/启动任务”分开，避免在
/// 持有 `self.state` 借用时再调用需要 `&mut self` 的收尾方法。
struct BackupRequest {
    action: BackupAction,
    is_export: bool,
    encrypt: bool,
    include_secrets: bool,
    encrypted_file: bool,
    passphrase: String,
    confirmed: String,
}

impl BackupRequest {
    /// 校验口令组合；返回给用户就地改正的文案。
    ///
    /// 文案顺序与迁移前一致：先拦“包含口令却未加密”，再逐项校验口令长度与
    /// 确认输入，导入加密文件只要求非空口令。
    fn validate(&self) -> Option<String> {
        if self.is_export && self.include_secrets && !self.encrypt {
            return Some(rust_i18n::t!("backup.error_secrets_require_encryption").to_string());
        }
        if self.is_export && self.encrypt {
            if self.passphrase.is_empty() {
                return Some(rust_i18n::t!("backup.error_empty").to_string());
            }
            if self.passphrase.chars().count() < 8 {
                return Some(rust_i18n::t!("backup.error_short").to_string());
            }
            if self.confirmed != self.passphrase {
                return Some(rust_i18n::t!("backup.error_mismatch").to_string());
            }
        }
        if !self.is_export && self.encrypted_file && self.passphrase.is_empty() {
            return Some(rust_i18n::t!("backup.error_empty").to_string());
        }
        None
    }

    /// 选择写入路径：导出弹保存对话框，导入复用打开时选中的文件。
    fn target_path(&self) -> Option<PathBuf> {
        match &self.action {
            BackupAction::Export => {
                let path = rfd::FileDialog::new()
                    .set_title(rust_i18n::t!("backup.export_title").to_string())
                    .set_file_name("transom-backup.json")
                    .add_filter("Transom Backup (JSON)", &["json"])
                    .add_filter("Transom Backup (YAML)", &["yaml", "yml"])
                    .save_file()?;
                Some(ensure_backup_extension(path))
            }
            BackupAction::Import(path) => Some(path.clone()),
        }
    }
}

/// 备份弹窗实体。
pub(crate) struct BackupPromptView {
    /// 未打开时为 `None`；每次打开都重新创建输入控件，避免残留上一次的口令。
    state: Option<BackupPromptState>,
    /// 备份服务句柄：实体自己完成后台任务，不经过外壳。
    backup: Arc<dyn BackupService>,
    /// 打开时注入的目录快照。
    catalog: SessionCatalog,
    /// 在途任务句柄；下一次提交覆盖旧句柄，实体销毁时随之取消。
    task: Option<Task<()>>,
}

impl EventEmitter<BackupPromptEvent> for BackupPromptView {}

impl BackupPromptView {
    /// 组装常驻实体；打开状态由 [`Self::open_export`]/[`Self::open_import`] 控制。
    pub(crate) fn new(backup: Arc<dyn BackupService>) -> Self {
        Self {
            state: None,
            backup,
            catalog: SessionCatalog::default(),
            task: None,
        }
    }

    /// 弹窗是否展开。
    pub(crate) fn is_open(&self) -> bool {
        self.state.is_some()
    }

    /// 当前打开期的只读状态；渲染与提交快照都从这里取。
    pub(super) fn prompt(&self) -> Option<&BackupPromptState> {
        self.state.as_ref()
    }

    /// 切换“加密导出”勾选。
    pub(super) fn set_encrypt(&mut self, checked: bool, cx: &mut Context<Self>) {
        if let Some(prompt) = self.state.as_mut() {
            prompt.encrypt = checked;
        }
        cx.notify();
    }

    /// 切换“包含口令”勾选；勾选时同步打开加密，提交时仍会校验。
    pub(super) fn set_include_secrets(&mut self, checked: bool, cx: &mut Context<Self>) {
        if let Some(prompt) = self.state.as_mut() {
            prompt.include_secrets = checked;
            if checked {
                prompt.encrypt = true;
            }
        }
        cx.notify();
    }

    /// 切换“文件已加密”勾选；勾选后把焦点交给口令框。
    pub(super) fn set_encrypted_file(
        &mut self,
        checked: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(prompt) = self.state.as_mut() {
            prompt.encrypted_file = checked;
        }
        if checked {
            self.focus_passphrase(window, cx);
        }
        cx.notify();
    }

    /// 注入本次打开使用的目录快照。
    pub(crate) fn set_catalog(&mut self, catalog: SessionCatalog) {
        self.catalog = catalog;
    }

    /// 打开导出弹窗；默认加密并把焦点交给口令框。
    ///
    /// `window` 是创建输入状态所必需的：`InputState` 的焦点句柄在构造时注册。
    pub(crate) fn open_export(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open(BackupAction::Export, true, window, cx);
    }

    /// 打开导入弹窗；默认按明文文件处理，因此不抢焦点。
    pub(crate) fn open_import(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open(BackupAction::Import(path), false, window, cx);
    }

    /// 关闭弹窗；在途任务继续跑完，收尾时发现状态已空会直接丢弃结果。
    pub(crate) fn close(&mut self, cx: &mut Context<Self>) {
        self.state = None;
        cx.notify();
    }

    /// 打开备份弹窗。
    ///
    /// 导出默认加密并把焦点交给口令框；导入默认按明文文件处理，不抢焦点，
    /// 等用户声明文件已加密再要口令。
    fn open(
        &mut self,
        action: BackupAction,
        is_export: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let passphrase = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder(rust_i18n::t!("backup.passphrase").to_string())
        });
        let confirm = is_export.then(|| {
            cx.new(|cx| {
                InputState::new(window, cx)
                    .masked(true)
                    .placeholder(rust_i18n::t!("backup.confirm").to_string())
            })
        });
        self.state = Some(BackupPromptState {
            action,
            passphrase: passphrase.clone(),
            confirm,
            encrypt: is_export,
            include_secrets: false,
            encrypted_file: false,
            error: None,
            busy: false,
        });
        if is_export {
            passphrase.update(cx, |state, cx| state.focus(window, cx));
        }
        cx.notify();
    }

    /// 校验口令后在后台执行导出或导入。
    ///
    /// Argon2 使用 64 MiB 内存，属于明显的阻塞计算；放到后台执行器避免
    /// 冻结窗口。任务句柄由实体持有，窗口关闭时随实体一起取消。
    pub(crate) fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(request) = self.request_snapshot(cx) else {
            return;
        };
        if let Some(message) = request.validate() {
            self.set_error(message, cx);
            return;
        }
        let Some(path) = request.target_path() else {
            return;
        };
        let BackupRequest {
            is_export,
            include_secrets,
            encrypt,
            encrypted_file,
            passphrase,
            ..
        } = request;
        // 口令只以 Zeroizing 形式进入任务；原字符串在解构时被移走，不再留副本。
        let passphrase = if is_export {
            encrypt.then(|| Zeroizing::new(passphrase))
        } else {
            encrypted_file.then(|| Zeroizing::new(passphrase))
        };
        if let Some(prompt) = self.state.as_mut() {
            prompt.busy = true;
            prompt.error = None;
        }
        let job = BackupJob {
            is_export,
            backup: self.backup.clone(),
            path,
            catalog: self.catalog.clone(),
            passphrase,
            include_secrets,
        };
        self.task = Some(self.spawn_backup_task(window, cx, job));
        cx.notify();
    }

    /// 读取当前输入；未打开或已在提交中时返回 `None`。
    fn request_snapshot(&self, cx: &App) -> Option<BackupRequest> {
        let prompt = self.state.as_ref()?;
        if prompt.busy {
            return None;
        }
        Some(BackupRequest {
            action: prompt.action.clone(),
            is_export: prompt.action.is_export(),
            encrypt: prompt.encrypt,
            include_secrets: prompt.include_secrets,
            encrypted_file: prompt.encrypted_file,
            passphrase: prompt.passphrase.read(cx).value().to_string(),
            confirmed: prompt
                .confirm
                .as_ref()
                .map(|input| input.read(cx).value().to_string())
                .unwrap_or_default(),
        })
    }

    /// 后台导出/导入任务：成功后回到 [`Self::finish`] 收尾。
    fn spawn_backup_task(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
        job: BackupJob,
    ) -> Task<()> {
        let BackupJob {
            is_export,
            backup,
            path,
            catalog,
            passphrase,
            include_secrets,
        } = job;
        if is_export {
            let options = BackupExportOptions {
                passphrase,
                include_secrets,
            };
            let target_path = path.clone();
            cx.spawn_in(window, async move |this, cx| {
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        let report = backup.export(&target_path, &catalog, &options)?;
                        Ok(BackupOutcome::Exported {
                            path: target_path,
                            report,
                        })
                    })
                    .await;
                let _ = this.update_in(cx, |view, window, cx| {
                    view.finish(result, window, cx);
                });
            })
        } else {
            let options = BackupImportOptions { passphrase };
            cx.spawn_in(window, async move |this, cx| {
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        let document = backup.import(&path, &options)?;
                        let (catalog, secrets) = document.into_parts();
                        let secrets = match secrets {
                            Some(secrets) => backup.restore_secrets(&secrets),
                            None => Ok(SecretRestoreReport::default()),
                        };
                        Ok(BackupOutcome::Imported { catalog, secrets })
                    })
                    .await;
                let _ = this.update_in(cx, |view, window, cx| {
                    view.finish(result, window, cx);
                });
            })
        }
    }

    /// 在弹窗上显示一条错误，不关闭弹窗以便用户就地改正。
    fn set_error(&mut self, message: String, cx: &mut Context<Self>) {
        if let Some(prompt) = self.state.as_mut() {
            prompt.error = Some(message);
        }
        cx.notify();
    }

    /// 后台任务收尾：成功关闭弹窗并上报，失败保留弹窗、自动纠正勾选。
    fn finish(
        &mut self,
        result: Result<BackupOutcome, AppError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 用户可能在任务运行期间关掉了弹窗（容器右上角关闭）；此时结果只丢弃，
        // 与迁移前“状态已空就提前返回”的行为一致。
        if self.state.is_none() {
            return;
        }
        match result {
            Ok(BackupOutcome::Exported { path, report }) => {
                self.state = None;
                cx.emit(BackupPromptEvent::Exported { path, report });
            }
            Ok(BackupOutcome::Imported { catalog, secrets }) => {
                self.state = None;
                cx.emit(BackupPromptEvent::Imported { catalog, secrets });
            }
            Err(error) => {
                let code = error.code;
                if let Some(prompt) = self.state.as_mut() {
                    prompt.busy = false;
                    prompt.error = Some(localized_error(&error));
                    // 文件类型与勾选不符时自动纠正勾选，省得用户在同一错误上反复试。
                    match code {
                        ErrorCode::BackupEncrypted => prompt.encrypted_file = true,
                        ErrorCode::BackupNotEncrypted => prompt.encrypted_file = false,
                        _ => {}
                    }
                }
                if code == ErrorCode::BackupEncrypted {
                    self.focus_passphrase(window, cx);
                }
                cx.emit(BackupPromptEvent::Error { error });
            }
        }
        cx.notify();
    }

    /// 把焦点交给口令框；供打开导出与“文件其实是加密的”错误纠正共用。
    pub(super) fn focus_passphrase(&self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(passphrase) = self.state.as_ref().map(|prompt| prompt.passphrase.clone()) else {
            return;
        };
        passphrase.update(cx, |state, cx| state.focus(window, cx));
    }
}
