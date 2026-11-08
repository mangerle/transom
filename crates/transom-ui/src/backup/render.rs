//! 备份弹窗主体渲染：选项、口令输入与页脚都经实体回调，不再转发外壳。
//!
//! 渲染期实体已被租借，因此这里只读 `state`；按钮回调经 `cx.listener` 回到
//! [`BackupPromptView`]，避免出现“渲染里再 `update` 自己”的双重租借。

use super::prompt::{BackupAction, BackupPromptState};
use super::view::BackupPromptView;
use crate::palette::Palette;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Disableable;
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::*;

impl BackupPromptView {
    /// 弹窗主体：标题、提示、按方向切换的选项与页脚。
    pub(crate) fn render_body(&mut self, cx: &mut Context<Self>, p: &Palette) -> AnyElement {
        let Some(prompt) = self.prompt() else {
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

        content = content.child(self.render_options(prompt, cx, p));

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

        let footer = self.render_footer(prompt, cx);

        crate::widgets::dialog_body(
            "backup-prompt-modal",
            title,
            Some(rust_i18n::t!("backup.dialog_hint").to_string()),
            content,
            Some(footer),
        )
    }

    /// 导出/导入选项：加密、包含口令与口令输入。
    fn render_options(
        &self,
        prompt: &BackupPromptState,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        if prompt.action.is_export() {
            self.render_export_options(prompt, cx, p)
        } else {
            self.render_import_options(prompt, cx, p)
        }
    }

    /// 导出选项：加密、包含口令与口令输入。
    fn render_export_options(
        &self,
        prompt: &BackupPromptState,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let mut content = v_flex().gap_3();

        content = content
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Checkbox::new("backup-encrypt")
                            .small()
                            .checked(prompt.encrypt)
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.set_encrypt(*checked, cx);
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
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.set_include_secrets(*checked, cx);
                            })),
                    )
                    .child(
                        div()
                            .text_sm()
                            .child(rust_i18n::t!("backup.include_secrets").to_string()),
                    ),
            );
        content = content.child(export_passphrase_fields(prompt, p));
        if prompt.include_secrets {
            content = content.child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("backup.include_secrets_hint").to_string()),
            );
        }
        content.into_any_element()
    }

    /// 导入选项：文件是否加密与解密口令。
    fn render_import_options(
        &self,
        prompt: &BackupPromptState,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let mut content = v_flex().gap_3();

        content = content.child(
            h_flex()
                .gap_2()
                .items_center()
                .child(
                    Checkbox::new("backup-file-encrypted")
                        .small()
                        .checked(prompt.encrypted_file)
                        .on_click(cx.listener(|this, checked: &bool, window, cx| {
                            this.set_encrypted_file(*checked, window, cx);
                        })),
                )
                .child(
                    div()
                        .text_sm()
                        .child(rust_i18n::t!("backup.file_encrypted").to_string()),
                ),
        );
        if prompt.encrypted_file {
            content = content.child(crate::widgets::form_field(
                rust_i18n::t!("backup.passphrase").to_string(),
                prompt.passphrase.clone(),
                p,
            ));
        }
        content.into_any_element()
    }

    /// 备份弹窗页脚：取消与提交，处理中禁用。
    fn render_footer(&self, prompt: &BackupPromptState, cx: &mut Context<Self>) -> AnyElement {
        let is_export = prompt.action.is_export();
        let busy = prompt.busy;
        let footer = h_flex()
            .gap_2()
            .child(
                Button::new("cancel-backup")
                    .ghost()
                    .disabled(busy)
                    .label(rust_i18n::t!("common.cancel").to_string())
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.close(cx))),
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
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.submit(window, cx);
                    })),
            );
        footer.into_any_element()
    }
}

/// 导出加密区：口令与确认输入，或未加密提示。
fn export_passphrase_fields(prompt: &BackupPromptState, p: &Palette) -> AnyElement {
    let mut content = v_flex().gap_3();
    if prompt.encrypt {
        content = content.child(crate::widgets::form_field(
            rust_i18n::t!("backup.passphrase").to_string(),
            prompt.passphrase.clone(),
            p,
        ));
        if let Some(confirm) = &prompt.confirm {
            content = content.child(crate::widgets::form_field(
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
    content.into_any_element()
}
