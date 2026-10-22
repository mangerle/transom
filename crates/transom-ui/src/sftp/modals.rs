//! SFTP 内置文本编辑器与 Linux 文件权限 (Chmod) 模态对话框视图渲染。

use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{Selectable, h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, Textarea};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::format_rwx_permissions;

impl AppShell {
    pub(crate) fn render_sftp_editor_modal(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let Some(modal) = self.sftp_editor_modal.as_ref() else {
            return div().into_any_element();
        };
        let file_name = modal.file_name.clone();
        let remote_path = modal.remote_path.clone();
        let textarea = modal.textarea.clone();
        let error = modal.error.clone();

        let content = v_flex()
            .gap_3()
            .child(Textarea::new(&textarea).h(px(380.0)).w_full())
            .when_some(error, |this, err| {
                this.child(div().text_xs().text_color(p.danger).child(err))
            });

        let footer = h_flex()
            .gap_2()
            .child(
                Button::new("cancel-sftp-editor")
                    .ghost()
                    .label(rust_i18n::t!("common.cancel").to_string())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.sftp_editor_modal = None;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("save-sftp-editor")
                    .primary()
                    .icon(IconName::Check)
                    .label("保存并上传")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.save_sftp_editor_modal(cx);
                    })),
            );

        let dialog = Self::dialog_frame(
            "sftp-editor-modal",
            format!("编辑远程文件: {file_name}"),
            Some(format!("远端路径: {remote_path}（保存后自动回传覆盖）")),
            content,
            Some(footer),
            px(720.0),
            p,
        );

        Self::overlay(dialog, p)
    }

    pub(crate) fn render_sftp_chmod_modal(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let Some(modal) = self.sftp_chmod_modal.as_ref() else {
            return div().into_any_element();
        };
        let file_name = modal.file_name.clone();
        let remote_path = modal.remote_path.clone();
        let perms = modal.permissions & 0o777;
        let octal_input = modal.octal_input.clone();
        let error = modal.error.clone();

        let groups = [
            (
                "所有者 (Owner)",
                (0o400, "chmod-ur"),
                (0o200, "chmod-uw"),
                (0o100, "chmod-ux"),
            ),
            (
                "所属组 (Group)",
                (0o040, "chmod-gr"),
                (0o020, "chmod-gw"),
                (0o010, "chmod-gx"),
            ),
            (
                "其他用户 (Others)",
                (0o004, "chmod-or"),
                (0o002, "chmod-ow"),
                (0o001, "chmod-ox"),
            ),
        ];

        let mut grid_rows: Vec<AnyElement> = Vec::with_capacity(3);
        for (group_title, (r_mask, r_id), (w_mask, w_id), (x_mask, x_id)) in groups {
            grid_rows.push(
                h_flex()
                    .py_2()
                    .px_3()
                    .rounded_md()
                    .border_1()
                    .border_color(p.border)
                    .bg(p.muted.opacity(0.4))
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .w(px(140.0))
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(group_title),
                    )
                    .child(
                        h_flex()
                            .gap_4()
                            .items_center()
                            .child(
                                Checkbox::new(r_id)
                                    .label("读取 (r)")
                                    .checked(perms & r_mask != 0)
                                    .on_change(cx.listener(move |this, &checked, window, cx| {
                                        this.sftp_toggle_chmod_bit(r_mask, checked, window, cx);
                                    })),
                            )
                            .child(
                                Checkbox::new(w_id)
                                    .label("写入 (w)")
                                    .checked(perms & w_mask != 0)
                                    .on_change(cx.listener(move |this, &checked, window, cx| {
                                        this.sftp_toggle_chmod_bit(w_mask, checked, window, cx);
                                    })),
                            )
                            .child(
                                Checkbox::new(x_id)
                                    .label("执行 (x)")
                                    .checked(perms & x_mask != 0)
                                    .on_change(cx.listener(move |this, &checked, window, cx| {
                                        this.sftp_toggle_chmod_bit(x_mask, checked, window, cx);
                                    })),
                            ),
                    )
                    .into_any_element(),
            );
        }

        let rwx_str = format_rwx_permissions(perms);
        let presets = [
            (0o755, "755"),
            (0o644, "644"),
            (0o700, "700"),
            (0o600, "600"),
        ];
        let mut preset_btns = h_flex().gap_1p5().items_center();
        for (mode_val, label_str) in presets {
            preset_btns = preset_btns.child(
                Button::new(format!("chmod-preset-{label_str}"))
                    .ghost()
                    .xsmall()
                    .label(label_str)
                    .selected(perms == mode_val)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.sftp_set_chmod_preset(mode_val, window, cx);
                    })),
            );
        }

        let content = v_flex()
            .gap_3()
            .children(grid_rows)
            .child(
                h_flex()
                    .gap_3()
                    .items_center()
                    .justify_between()
                    .pt_1()
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(p.muted_foreground)
                                    .child("八进制权限码:"),
                            )
                            .child(div().w(px(90.0)).child(Input::new(&octal_input).w_full()))
                            .child(
                                div()
                                    .text_xs()
                                    .px_2()
                                    .py_1()
                                    .rounded_sm()
                                    .bg(p.muted)
                                    .text_color(p.muted_foreground)
                                    .child(rwx_str),
                            ),
                    )
                    .child(preset_btns),
            )
            .when_some(error, |this, err| {
                this.child(div().text_xs().text_color(p.danger).child(err))
            });

        let footer = h_flex()
            .gap_2()
            .child(
                Button::new("cancel-sftp-chmod")
                    .ghost()
                    .label(rust_i18n::t!("common.cancel").to_string())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.sftp_chmod_modal = None;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("save-sftp-chmod")
                    .primary()
                    .icon(IconName::Check)
                    .label("应用权限")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.save_sftp_chmod_modal(cx);
                    })),
            );

        let dialog = Self::dialog_frame(
            "sftp-chmod-modal",
            format!("修改文件权限 (Chmod): {file_name}"),
            Some(format!("远端路径: {remote_path}")),
            content,
            Some(footer),
            px(520.0),
            p,
        );

        Self::overlay(dialog, p)
    }

    pub(crate) fn render_sftp_input_modal(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let Some(modal) = self.sftp_input_modal.as_ref() else {
            return div().into_any_element();
        };
        let title = modal.title.clone();
        let prompt = modal.prompt.clone();
        let input = modal.input.clone();
        let error = modal.error.clone();

        let content = v_flex()
            .gap_3()
            .child(Input::new(&input).w_full())
            .when_some(error, |this, err| {
                this.child(div().text_xs().text_color(p.danger).child(err))
            });

        let footer = h_flex()
            .gap_2()
            .child(
                Button::new("cancel-sftp-input")
                    .ghost()
                    .label(rust_i18n::t!("common.cancel").to_string())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.sftp_input_modal = None;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("confirm-sftp-input")
                    .primary()
                    .icon(IconName::Check)
                    .label(rust_i18n::t!("common.confirm").to_string())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.commit_sftp_input_modal(cx);
                    })),
            );

        let dialog = Self::dialog_frame(
            "sftp-input-modal",
            title,
            Some(prompt),
            content,
            Some(footer),
            px(420.0),
            p,
        );

        Self::overlay(dialog, p)
    }
}
