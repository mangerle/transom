//! 会话目录备份的导入导出交互。
//!
//! 加密与否由用户在弹窗里勾选：加密备份用 Argon2id 派生密钥并以
//! XChaCha20-Poly1305 认证加密写入信封；明文备份直接落一份 JSON/YAML，
//! 便于人工查看与迁移，但文件里的主机名、用户名是明文。

use crate::app::{AppShell, BackupAction, BackupPromptState};
use crate::palette::{Palette, localized_error};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Disableable;
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::InputState;
use gpui_kit::*;
use std::path::PathBuf;
use transom_core::{
    AppError, BackupExportOptions, BackupExportReport, BackupImportOptions, ErrorCode,
    SecretRestoreReport, SessionCatalog,
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
    pub(crate) fn open_backup_export(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = rfd::FileDialog::new()
            .set_title(rust_i18n::t!("backup.export_title").to_string())
            .set_file_name("transom-backup.json")
            .add_filter("Transom Backup (JSON)", &["json"])
            .add_filter("Transom Backup (YAML)", &["yaml", "yml"])
            .save_file()
        else {
            return;
        };
        let path = ensure_backup_extension(path);
        self.open_backup_prompt(BackupAction::Export(path), true, window, cx);
    }

    pub(crate) fn open_backup_import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = rfd::FileDialog::new()
            .set_title(rust_i18n::t!("backup.import_title").to_string())
            .add_filter("Transom Backup", &["json", "yaml", "yml"])
            .pick_file()
        else {
            return;
        };
        self.open_backup_prompt(BackupAction::Import(path), false, window, cx);
    }

    /// 打开备份弹窗。
    ///
    /// 导出默认加密并把焦点交给口令框；导入默认按明文文件处理，因此不抢
    /// 焦点，等用户声明文件已加密再要口令。
    fn open_backup_prompt(
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
        self.backup_prompt = Some(BackupPromptState {
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
    /// 冻结窗口。任务句柄保存在 `_backup_task`，窗口关闭时自动取消。
    pub(crate) fn submit_backup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(prompt) = self.backup_prompt.as_ref() else {
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
        if let Some(prompt) = self.backup_prompt.as_mut() {
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
        self._backup_task = Some(task);
        cx.notify();
    }

    /// 在弹窗上显示一条错误，不关闭弹窗以便用户就地改正。
    fn set_backup_error(&mut self, message: String, cx: &mut Context<Self>) {
        if let Some(prompt) = self.backup_prompt.as_mut() {
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
        let Some(prompt) = self.backup_prompt.as_ref() else {
            return;
        };
        match result {
            Ok(BackupOutcome::Exported(report)) => {
                let path = prompt.action.path().display().to_string();
                self.backup_prompt = None;
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
                let summary = self.catalog.merge(catalog);
                let saved = self.persist_catalog();
                self.refresh_tab_sessions();
                self.backup_prompt = None;
                let mut message = rust_i18n::t!(
                    "backup.imported",
                    added = summary.added_sessions,
                    updated = summary.updated_sessions,
                    folders = summary.added_folders
                )
                .to_string();
                message.push_str(&secret_restore_note(secrets));
                if saved {
                    self.status = Some(message);
                }
            }
            Err(error) => {
                let code = error.code;
                if let Some(prompt) = self.backup_prompt.as_mut() {
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
                    && let Some(prompt) = self.backup_prompt.as_ref()
                {
                    let passphrase = prompt.passphrase.clone();
                    passphrase.update(cx, |state, cx| state.focus(window, cx));
                }
                cx.notify();
            }
        }
        cx.notify();
    }

    pub(crate) fn render_backup_prompt(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let Some(prompt) = self.backup_prompt.as_ref() else {
            return div().into_any_element();
        };
        let is_export = prompt.action.is_export();
        let title = if is_export {
            rust_i18n::t!("backup.export_title").to_string()
        } else {
            rust_i18n::t!("backup.import_title").to_string()
        };
        let path = prompt.action.path().display().to_string();
        let busy = prompt.busy;
        let error = prompt.error.clone();

        let mut content = v_flex()
            .gap_3()
            .child(div().text_xs().text_color(p.muted_foreground).child(path))
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("backup.hint").to_string()),
            );

        if is_export {
            content = content
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(
                            Checkbox::new("backup-encrypt")
                                .small()
                                .checked(prompt.encrypt)
                                .on_click(cx.listener(|this, &checked, _, cx| {
                                    if let Some(prompt) = this.backup_prompt.as_mut() {
                                        prompt.encrypt = checked;
                                    }
                                    cx.notify();
                                })),
                        )
                        .child(
                            div()
                                .text_sm()
                                .child(rust_i18n::t!("backup.encrypt").to_string()),
                        ),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(
                            Checkbox::new("backup-include-secrets")
                                .small()
                                .checked(prompt.include_secrets)
                                .on_click(cx.listener(|this, &checked, _, cx| {
                                    if let Some(prompt) = this.backup_prompt.as_mut() {
                                        prompt.include_secrets = checked;
                                    }
                                    cx.notify();
                                })),
                        )
                        .child(
                            div()
                                .text_sm()
                                .child(rust_i18n::t!("backup.include_secrets").to_string()),
                        ),
                );
            if prompt.encrypt {
                content = content.child(Self::form_field(
                    rust_i18n::t!("backup.passphrase").to_string(),
                    prompt.passphrase.clone(),
                    p,
                ));
                if let Some(confirm) = &prompt.confirm {
                    content = content.child(Self::form_field(
                        rust_i18n::t!("backup.confirm").to_string(),
                        confirm.clone(),
                        p,
                    ));
                }
                content = content.child(
                    div()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(rust_i18n::t!("backup.passphrase_hint").to_string()),
                );
            } else {
                content = content.child(
                    div()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(rust_i18n::t!("backup.plain_hint").to_string()),
                );
            }
            if prompt.include_secrets {
                content = content.child(
                    div()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(rust_i18n::t!("backup.include_secrets_hint").to_string()),
                );
            }
        } else {
            content = content.child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Checkbox::new("backup-file-encrypted")
                            .small()
                            .checked(prompt.encrypted_file)
                            .on_click(cx.listener(|this, &checked, window, cx| {
                                if let Some(prompt) = this.backup_prompt.as_mut() {
                                    prompt.encrypted_file = checked;
                                }
                                if checked && let Some(prompt) = this.backup_prompt.as_ref() {
                                    let passphrase = prompt.passphrase.clone();
                                    passphrase.update(cx, |state, cx| state.focus(window, cx));
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .text_sm()
                            .child(rust_i18n::t!("backup.file_encrypted").to_string()),
                    ),
            );
            if prompt.encrypted_file {
                content = content.child(Self::form_field(
                    rust_i18n::t!("backup.passphrase").to_string(),
                    prompt.passphrase.clone(),
                    p,
                ));
            }
        }

        if busy {
            content = content.child(
                div()
                    .text_sm()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("backup.processing").to_string()),
            );
        }
        if let Some(error) = error {
            content = content.child(div().text_sm().text_color(p.danger).child(error));
        }

        let footer = h_flex()
            .gap_2()
            .child(
                Button::new("cancel-backup")
                    .ghost()
                    .disabled(busy)
                    .label(rust_i18n::t!("common.cancel").to_string())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.backup_prompt = None;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("submit-backup")
                    .primary()
                    .disabled(busy)
                    .label(if is_export {
                        rust_i18n::t!("backup.export_action").to_string()
                    } else {
                        rust_i18n::t!("backup.import_action").to_string()
                    })
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.submit_backup(window, cx);
                    })),
            );

        let dialog = Self::dialog_frame(
            "backup-prompt-modal",
            title,
            Some(rust_i18n::t!("backup.dialog_hint").to_string()),
            content,
            Some(footer),
            px(520.0),
            p,
        );
        Self::overlay(dialog, p)
    }
}

/// 口令写回结果的附加说明。
pub(crate) fn secret_restore_note(secrets: Result<SecretRestoreReport, AppError>) -> String {
    match secrets {
        Ok(report) if report.restored > 0 || report.kept > 0 => format!(
            " {}",
            rust_i18n::t!(
                "backup.imported_secrets",
                restored = report.restored,
                kept = report.kept
            )
        ),
        Ok(_) => String::new(),
        Err(error) => format!(
            " {}: {}",
            rust_i18n::t!("backup.imported_secrets_failed"),
            localized_error(&error)
        ),
    }
}

fn ensure_backup_extension(path: PathBuf) -> PathBuf {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("json" | "yaml" | "yml") => path,
        _ => path.with_extension("json"),
    }
}
