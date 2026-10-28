//! 会话目录备份的导入导出交互。
//!
//! 加密与否由用户在弹窗里勾选：加密备份用 Argon2id 派生密钥并以
//! XChaCha20-Poly1305 认证加密写入信封；明文备份直接落一份 JSON/YAML，
//! 便于人工查看与迁移，但文件里的主机名、用户名是明文。

mod helpers;
mod prompt;

pub(crate) use helpers::secret_restore_note;
pub(crate) use prompt::BackupPromptState;

use crate::app::AppShell;
use crate::palette::localized_error;
use gpui_kit::*;
use transom_core::{
    AppError, BackupExportOptions, BackupExportReport, BackupImportOptions, ErrorCode,
    MergePreference, SecretRestoreReport, SessionCatalog, merge_catalogs,
};
use zeroize::Zeroizing;

/// 后台备份任务的结果。
enum BackupOutcome {
    /// 导出完成，附带统计。
    Exported(BackupExportReport),
    /// 导入完成：目录快照交由界面合并，口令写回结果单独汇报。
    Imported {
        catalog: SessionCatalog,
        /// 口令写回结果；钥匙串不可用时携带错误，会话导入本身不受影响。
        secrets: Result<SecretRestoreReport, AppError>,
    },
}

impl AppShell {
    /// 校验口令后在后台执行导出或导入。
    ///
    /// Argon2 使用 64 MiB 内存，属于明显的阻塞计算；放到后台执行器避免
    /// 冻结窗口。任务句柄保存在 `_backup_task`，窗口关闭时自动取消。
    pub(crate) fn submit_backup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(prompt) = self.modals.backup_prompt.as_ref() else {
            return;
        };
        if prompt.busy {
            return;
        }
        let is_export = prompt.action.is_export();
        let encrypt = prompt.encrypt;
        let include_secrets = prompt.include_secrets;
        let encrypted_file = prompt.encrypted_file;
        let value = prompt.passphrase.read(cx).value().to_string();
        let confirmed = prompt
            .confirm
            .as_ref()
            .map(|input| input.read(cx).value().to_string())
            .unwrap_or_default();

        if is_export && include_secrets && !encrypt {
            self.set_backup_error(
                rust_i18n::t!("backup.error_secrets_require_encryption").to_string(),
                cx,
            );
            return;
        }
        if is_export && encrypt {
            if value.is_empty() {
                self.set_backup_error(rust_i18n::t!("backup.error_empty").to_string(), cx);
                return;
            }
            if value.chars().count() < 8 {
                self.set_backup_error(rust_i18n::t!("backup.error_short").to_string(), cx);
                return;
            }
            if confirmed != value {
                self.set_backup_error(rust_i18n::t!("backup.error_mismatch").to_string(), cx);
                return;
            }
        }
        if !is_export && encrypted_file && value.is_empty() {
            self.set_backup_error(rust_i18n::t!("backup.error_empty").to_string(), cx);
            return;
        }

        let path = prompt.action.path().to_path_buf();
        let backup = self.services.backup.clone();
        if let Some(prompt) = self.modals.backup_prompt.as_mut() {
            prompt.busy = true;
            prompt.error = None;
        }

        let task = if is_export {
            let options = BackupExportOptions {
                passphrase: encrypt.then(|| Zeroizing::new(value)),
                include_secrets,
            };
            let catalog = self.catalog.clone();
            cx.spawn_in(window, async move |this, cx| {
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        backup
                            .export(&path, &catalog, &options)
                            .map(BackupOutcome::Exported)
                    })
                    .await;
                let _ = this.update_in(cx, |this, window, cx| {
                    this.finish_backup(result, window, cx);
                });
            })
        } else {
            let options = BackupImportOptions {
                passphrase: encrypted_file.then(|| Zeroizing::new(value)),
            };
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
                let _ = this.update_in(cx, |this, window, cx| {
                    this.finish_backup(result, window, cx);
                });
            })
        };
        self.tasks.backup = Some(task);
        cx.notify();
    }

    /// 在弹窗上显示一条错误，不关闭弹窗以便用户就地改正。
    fn set_backup_error(&mut self, message: String, cx: &mut Context<Self>) {
        if let Some(prompt) = self.modals.backup_prompt.as_mut() {
            prompt.error = Some(message);
        }
        cx.notify();
    }

    fn finish_backup(
        &mut self,
        result: Result<BackupOutcome, AppError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(prompt) = self.modals.backup_prompt.as_ref() else {
            return;
        };
        match result {
            Ok(BackupOutcome::Exported(report)) => {
                let path = prompt.action.path().display().to_string();
                self.modals.backup_prompt = None;
                let mut message = rust_i18n::t!("backup.exported", path = path).to_string();
                if report.secrets > 0 {
                    message.push(' ');
                    message.push_str(
                        rust_i18n::t!("backup.exported_secrets", count = report.secrets).as_ref(),
                    );
                }
                self.status = Some(message);
            }
            Ok(BackupOutcome::Imported { catalog, secrets }) => {
                let (merged, report) =
                    merge_catalogs(None, &self.catalog, &catalog, MergePreference::Remote);
                self.catalog = merged;
                let saved = self.persist_catalog();
                self.refresh_tab_sessions();
                self.modals.backup_prompt = None;
                let mut message = rust_i18n::t!(
                    "backup.imported",
                    added = report.sessions_added,
                    updated = report.sessions_updated,
                    folders = report.folders_added
                )
                .to_string();
                if report.conflicts > 0 {
                    message.push(' ');
                    message.push_str(
                        rust_i18n::t!("sync.merge_conflicts", count = report.conflicts).as_ref(),
                    );
                }
                message.push_str(&secret_restore_note(secrets));
                if saved {
                    self.status = Some(message);
                }
            }
            Err(error) => {
                let code = error.code;
                if let Some(prompt) = self.modals.backup_prompt.as_mut() {
                    prompt.busy = false;
                    prompt.error = Some(localized_error(&error));
                    // 文件类型与勾选不符时自动纠正勾选，省得用户在同一错误上反复试。
                    match code {
                        ErrorCode::BackupEncrypted => prompt.encrypted_file = true,
                        ErrorCode::BackupNotEncrypted => prompt.encrypted_file = false,
                        _ => {}
                    }
                }
                if code == ErrorCode::BackupEncrypted
                    && let Some(prompt) = self.modals.backup_prompt.as_ref()
                {
                    let passphrase = prompt.passphrase.clone();
                    passphrase.update(cx, |state, cx| state.focus(window, cx));
                }
                cx.notify();
            }
        }
        cx.notify();
    }
}
