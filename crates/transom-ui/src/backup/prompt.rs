//! 备份弹窗交互与 UI 渲染组件。

use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Disableable;
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::InputState;
use gpui_kit::*;
use std::path::PathBuf;

/// 备份导出/导入动作及其目标路径。
#[derive(Debug, Clone)]
pub(crate) enum BackupAction {
    Export,
    Import(PathBuf),
}

impl BackupAction {
    pub(crate) fn is_export(&self) -> bool {
        matches!(self, Self::Export)
    }
}

/// 备份口令弹窗的输入与校验状态。
///
/// `encrypt`/`include_secrets`/`encrypted_file` 分别是用户选择、导出内容
/// 范围与所选文件自身格式，三者语义不同，不能合并成互斥枚举。
pub(crate) struct BackupPromptState {
    pub(crate) action: BackupAction,
    pub(crate) passphrase: Entity<InputState>,
    pub(crate) confirm: Option<Entity<InputState>>,
    pub(crate) encrypt: bool,
    pub(crate) include_secrets: bool,
    pub(crate) encrypted_file: bool,
    pub(crate) error: Option<String>,
    pub(crate) busy: bool,
}

impl AppShell {
    pub(crate) fn open_backup_export(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_backup_prompt(BackupAction::Export, true, window, cx);
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
        self.modals.backup_prompt = Some(BackupPromptState {
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

    pub(crate) fn render_backup_prompt(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let Some(prompt) = self.modals.backup_prompt.as_ref() else {
            return div().into_any_element();
        };
        let is_export = prompt.action.is_export();
        let title = if is_export {
            rust_i18n::t!("backup.export_title").to_string()
        } else {
            rust_i18n::t!("backup.import_title").to_string()
        };
        let busy = prompt.busy;
        let error = prompt.error.clone();

        let mut content = v_flex().gap_3();

        if let BackupAction::Import(path) = &prompt.action {
            content = content.child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(path.display().to_string()),
            );
        }

        content = content.child(
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
                                    if let Some(prompt) = this.modals.backup_prompt.as_mut() {
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
                                    if let Some(prompt) = this.modals.backup_prompt.as_mut() {
                                        prompt.include_secrets = checked;
                                        // 口令必须加密落盘：勾选包含口令时同步打开加密，
                                        // 用户仍可手动取消，但提交时会被明确拦下。
                                        if checked {
                                            prompt.encrypt = true;
                                        }
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
                                if let Some(prompt) = this.modals.backup_prompt.as_mut() {
                                    prompt.encrypted_file = checked;
                                }
                                if checked && let Some(prompt) = this.modals.backup_prompt.as_ref()
                                {
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
                        this.modals.backup_prompt = None;
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
