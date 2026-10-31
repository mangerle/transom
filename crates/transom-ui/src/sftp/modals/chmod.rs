//! SFTP 弹窗视图。

use super::common::*;

impl AppShell {
    pub(crate) fn render_sftp_chmod_body(
        &self,
        app: &Entity<AppShell>,
        _cx: &App,
        p: &Palette,
    ) -> AnyElement {
        let Some(modal) = self.sftp_modals.chmod.as_ref() else {
            return div().into_any_element();
        };
        let file_name = modal.file_name.clone();
        let remote_path = modal.remote_path.clone();
        let perms = modal.permissions & 0o777;
        let octal_input = modal.octal_input.clone();
        let error = modal.error.clone();

        let groups = [
            (
                rust_i18n::t!("sftp.chmod.group_owner").to_string(),
                (0o400, "chmod-ur"),
                (0o200, "chmod-uw"),
                (0o100, "chmod-ux"),
            ),
            (
                rust_i18n::t!("sftp.chmod.group_group").to_string(),
                (0o040, "chmod-gr"),
                (0o020, "chmod-gw"),
                (0o010, "chmod-gx"),
            ),
            (
                rust_i18n::t!("sftp.chmod.group_others").to_string(),
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
                    .rounded(p.radius.md)
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
                                    .label(rust_i18n::t!("sftp.chmod.read").to_string())
                                    .checked(perms & r_mask != 0)
                                    .on_change(crate::dialog_host::on_event(
                                        app,
                                        move |this, checked: &bool, window, cx| {
                                            this.sftp_toggle_chmod_bit(
                                                r_mask, *checked, window, cx,
                                            );
                                        },
                                    )),
                            )
                            .child(
                                Checkbox::new(w_id)
                                    .label(rust_i18n::t!("sftp.chmod.write").to_string())
                                    .checked(perms & w_mask != 0)
                                    .on_change(crate::dialog_host::on_event(
                                        app,
                                        move |this, checked: &bool, window, cx| {
                                            this.sftp_toggle_chmod_bit(
                                                w_mask, *checked, window, cx,
                                            );
                                        },
                                    )),
                            )
                            .child(
                                Checkbox::new(x_id)
                                    .label(rust_i18n::t!("sftp.chmod.execute").to_string())
                                    .checked(perms & x_mask != 0)
                                    .on_change(crate::dialog_host::on_event(
                                        app,
                                        move |this, checked: &bool, window, cx| {
                                            this.sftp_toggle_chmod_bit(
                                                x_mask, *checked, window, cx,
                                            );
                                        },
                                    )),
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
                    .on_click(crate::dialog_host::on_event(
                        app,
                        move |this, _: &ClickEvent, window, cx| {
                            this.sftp_set_chmod_preset(mode_val, window, cx);
                        },
                    )),
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
                                    .child(rust_i18n::t!("sftp.chmod.octal_label").to_string()),
                            )
                            .child(div().w(px(90.0)).child(Input::new(&octal_input).w_full()))
                            .child(
                                div()
                                    .text_xs()
                                    .px_2()
                                    .py_1()
                                    .rounded(p.radius.sm)
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
            .child(crate::widgets::dialog_cancel_button(
                "cancel-sftp-chmod",
                rust_i18n::t!("common.cancel").to_string(),
                crate::dialog_host::on_event(app, |this, _: &ClickEvent, window, cx| {
                    this.sftp_modals.chmod = None;
                    this.focus_active_sftp_panel(window, cx);
                    cx.notify();
                }),
            ))
            .child(
                Button::new("save-sftp-chmod")
                    .primary()
                    .icon(IconName::Check)
                    .label(rust_i18n::t!("sftp.chmod.apply").to_string())
                    .on_click(crate::dialog_host::on_event(
                        app,
                        |this, _: &ClickEvent, window, cx| {
                            this.save_sftp_chmod_modal(window, cx);
                        },
                    )),
            );

        crate::widgets::dialog_body(
            "sftp-chmod-modal",
            rust_i18n::t!("sftp.chmod.title", name = file_name.as_str()).to_string(),
            Some(rust_i18n::t!("sftp.chmod.remote_path", path = remote_path.as_str()).to_string()),
            content,
            Some(footer),
        )
    }
}
