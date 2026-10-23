//! 会话目录加密备份的导入导出交互。

use crate::app::{AppShell, BackupAction, BackupPromptState};
use crate::palette::{Palette, localized_error};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Disableable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::InputState;
use gpui_kit::*;
use std::path::PathBuf;
use transom_core::{AppError, SessionCatalog};
use zeroize::Zeroizing;

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

    fn open_backup_prompt(
        &mut self,
        action: BackupAction,
        require_confirm: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let passphrase = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder(rust_i18n::t!("backup.passphrase").to_string())
        });
        passphrase.update(cx, |state, cx| state.focus(window, cx));
        let confirm = require_confirm.then(|| {
            cx.new(|cx| {
                InputState::new(window, cx)
                    .masked(true)
                    .placeholder(rust_i18n::t!("backup.confirm").to_string())
            })
        });
        self.backup_prompt = Some(BackupPromptState {
            action,
            passphrase,
            confirm,
            error: None,
            busy: false,
        });
        cx.notify();
    }

    /// 校验口令后在后台执行 Argon2 派生与加解密。
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
        let value = prompt.passphrase.read(cx).value().to_string();
        if value.is_empty() {
            if let Some(prompt) = self.backup_prompt.as_mut() {
                prompt.error = Some(rust_i18n::t!("backup.error_empty").to_string());
            }
            cx.notify();
            return;
        }
        if is_export {
            if value.chars().count() < 8 {
                if let Some(prompt) = self.backup_prompt.as_mut() {
                    prompt.error = Some(rust_i18n::t!("backup.error_short").to_string());
                }
                cx.notify();
                return;
            }
            let confirmed = prompt
                .confirm
                .as_ref()
                .map(|input| input.read(cx).value().to_string())
                .unwrap_or_default();
            if confirmed != value {
                if let Some(prompt) = self.backup_prompt.as_mut() {
                    prompt.error = Some(rust_i18n::t!("backup.error_mismatch").to_string());
                }
                cx.notify();
                return;
            }
        }

        let action = prompt.action.clone();
        let passphrase = Zeroizing::new(value);
        let catalog = self.catalog.clone();
        let backup = self.services.backup.clone();
        if let Some(prompt) = self.backup_prompt.as_mut() {
            prompt.busy = true;
            prompt.error = None;
        }
        let task = cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    match action {
                        BackupAction::Export(path) => {
                            backup.export(&path, &catalog, &passphrase).map(|()| None)
                        }
                        BackupAction::Import(path) => backup.import(&path, &passphrase).map(Some),
                    }
                })
                .await;
            let _ = this.update_in(cx, |this, _window, cx| {
                this.finish_backup(result, cx);
            });
        });
        self._backup_task = Some(task);
        cx.notify();
    }

    fn finish_backup(
        &mut self,
        result: Result<Option<SessionCatalog>, AppError>,
        cx: &mut Context<Self>,
    ) {
        let Some(prompt) = self.backup_prompt.as_ref() else {
            return;
        };
        match result {
            Ok(None) => {
                let path = prompt.action.path().display().to_string();
                self.backup_prompt = None;
                self.status = Some(rust_i18n::t!("backup.exported", path = path).to_string());
            }
            Ok(Some(imported)) => {
                let summary = self.catalog.merge(imported);
                let saved = self.persist_catalog();
                self.refresh_tab_sessions();
                self.backup_prompt = None;
                if saved {
                    self.status = Some(
                        rust_i18n::t!(
                            "backup.imported",
                            added = summary.added_sessions,
                            updated = summary.updated_sessions,
                            folders = summary.added_folders
                        )
                        .to_string(),
                    );
                }
            }
            Err(error) => {
                if let Some(prompt) = self.backup_prompt.as_mut() {
                    prompt.busy = false;
                    prompt.error = Some(localized_error(&error));
                }
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
            )
            .child(Self::form_field(
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
        if is_export {
            content = content.child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("backup.passphrase_hint").to_string()),
            );
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
