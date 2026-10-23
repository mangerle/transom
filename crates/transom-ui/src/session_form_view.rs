//! 新建/编辑会话配置表单模态弹窗渲染模块。

use crate::app::AppShell;
use crate::palette::Palette;
use crate::session_form::{AuthChoice, ConnectionChoice};
use crate::session_form_serial::render_serial_fields;
use gpui_kit::assets::IconName;
use gpui_kit::base::{Selectable, h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Textarea;
use gpui_kit::component::switch::Switch;
use gpui_kit::*;

impl AppShell {
    pub(crate) fn render_session_form(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let Some(form) = self.form.as_ref() else {
            return div().into_any_element();
        };
        let title = if form.base.is_some() {
            rust_i18n::t!("form.edit_title").to_string()
        } else {
            rust_i18n::t!("form.new_title").to_string()
        };

        let auth_password = if form.auth == AuthChoice::Password {
            Button::new("auth-password")
                .primary()
                .small()
                .label(rust_i18n::t!("form.auth_password").to_string())
        } else {
            Button::new("auth-password")
                .small()
                .label(rust_i18n::t!("form.auth_password").to_string())
        }
        .on_click(cx.listener(|this, _, _, cx| {
            if let Some(form) = this.form.as_mut() {
                form.auth = AuthChoice::Password;
            }
            cx.notify();
        }));
        let auth_key = if form.auth == AuthChoice::PrivateKey {
            Button::new("auth-key")
                .primary()
                .small()
                .label(rust_i18n::t!("form.auth_key").to_string())
        } else {
            Button::new("auth-key")
                .small()
                .label(rust_i18n::t!("form.auth_key").to_string())
        }
        .on_click(cx.listener(|this, _, _, cx| {
            if let Some(form) = this.form.as_mut() {
                form.auth = AuthChoice::PrivateKey;
            }
            cx.notify();
        }));
        let auth_agent = if form.auth == AuthChoice::Agent {
            Button::new("auth-agent")
                .primary()
                .small()
                .label(rust_i18n::t!("form.auth_agent").to_string())
        } else {
            Button::new("auth-agent")
                .small()
                .label(rust_i18n::t!("form.auth_agent").to_string())
        }
        .on_click(cx.listener(|this, _, _, cx| {
            if let Some(form) = this.form.as_mut() {
                form.auth = AuthChoice::Agent;
            }
            cx.notify();
        }));

        // 内容区自带有限高度的滚动区：标题、错误提示与操作按钮由 `dialog_frame`
        // 固定在外壳上，字段再多也不会把它们移出视野。
        // 这里不能改用 `flex_1`：外壳高度是自适应的，flex basis 归零会把整块内容压成 0 高。
        let mut content = v_flex()
            .id(ElementId::Name("session-form-body".into()))
            .max_h(px(460.0))
            .overflow_y_scroll()
            .gap_3()
            .child(
                h_flex()
                    .gap_3()
                    .child(div().flex_1().child(Self::form_field(
                        rust_i18n::t!("form.name").to_string(),
                        form.name.clone(),
                        p,
                    )))
                    .child(div().flex_1().child(Self::form_field(
                        rust_i18n::t!("form.folder").to_string(),
                        form.folder_path.clone(),
                        p,
                    ))),
            )
            .child(
                v_flex()
                    .gap_2()
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child(rust_i18n::t!("form.connection_type").to_string()),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("conn-ssh")
                                    .small()
                                    .label("SSH")
                                    .selected(form.connection == ConnectionChoice::Ssh)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if let Some(form) = this.form.as_mut() {
                                            form.connection = ConnectionChoice::Ssh;
                                        }
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("conn-serial")
                                    .small()
                                    .label(rust_i18n::t!("form.conn_serial").to_string())
                                    .selected(form.connection == ConnectionChoice::Serial)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        let needs_ports = if let Some(form) = this.form.as_mut() {
                                            form.connection = ConnectionChoice::Serial;
                                            form.serial_ports.is_empty()
                                        } else {
                                            false
                                        };
                                        if needs_ports {
                                            this.refresh_serial_ports(window, cx);
                                        }
                                        cx.notify();
                                    })),
                            ),
                    ),
            );

        match form.connection {
            ConnectionChoice::Ssh => {
                content = content
                    .child(
                        h_flex()
                            .gap_3()
                            .items_end()
                            .child(div().flex_1().child(Self::form_field(
                                rust_i18n::t!("form.host").to_string(),
                                form.host.clone(),
                                p,
                            )))
                            .child(div().w(px(110.0)).child(Self::form_field(
                                rust_i18n::t!("form.port").to_string(),
                                form.port.clone(),
                                p,
                            ))),
                    )
                    .child(Self::form_field(
                        rust_i18n::t!("form.username").to_string(),
                        form.username.clone(),
                        p,
                    ))
                    .child(
                        v_flex()
                            .gap_2()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(p.muted_foreground)
                                    .child(rust_i18n::t!("form.auth").to_string()),
                            )
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(auth_password)
                                    .child(auth_key)
                                    .child(auth_agent),
                            ),
                    );
                match form.auth {
                    AuthChoice::Password => {
                        content = content.child(
                            h_flex()
                                .justify_between()
                                .items_center()
                                .py_1()
                                .child(
                                    div()
                                        .text_sm()
                                        .child(rust_i18n::t!("form.save_password").to_string()),
                                )
                                .child(
                                    Switch::new("toggle-save-password")
                                        .small()
                                        .checked(form.save_password)
                                        .on_click(cx.listener(|this, &checked, _, cx| {
                                            if let Some(form) = this.form.as_mut() {
                                                form.save_password = checked;
                                            }
                                            cx.notify();
                                        })),
                                ),
                        );
                        // 只有保存到系统钥匙串时表单才收这份密码：不勾选就没有存放处，
                        // 交由连接时的认证询问负责，消除"填了却被丢弃"的歧义。
                        content = if form.save_password {
                            content.child(Self::secret_field(
                                rust_i18n::t!("form.password").to_string(),
                                form.password.clone(),
                                p,
                            ))
                        } else {
                            content.child(Self::credential_deferred_hint(p))
                        };
                    }
                    AuthChoice::PrivateKey => {
                        content = content.child(
                            h_flex()
                                .gap_2()
                                .items_end()
                                .child(div().flex_1().child(Self::form_field(
                                    rust_i18n::t!("form.key_path").to_string(),
                                    form.key_path.clone(),
                                    p,
                                )))
                                .child(
                                    Button::new("browse-key-path")
                                        .ghost()
                                        .small()
                                        .icon(IconName::FolderOpen)
                                        .label(rust_i18n::t!("form.browse").to_string())
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.pick_private_key_path(window, cx);
                                        })),
                                ),
                        );
                        content = content.child(
                            h_flex()
                                .justify_between()
                                .items_center()
                                .py_1()
                                .child(
                                    div()
                                        .text_sm()
                                        .child(rust_i18n::t!("form.save_passphrase").to_string()),
                                )
                                .child(
                                    Switch::new("toggle-save-passphrase")
                                        .small()
                                        .checked(form.save_passphrase)
                                        .on_click(cx.listener(|this, &checked, _, cx| {
                                            if let Some(form) = this.form.as_mut() {
                                                form.save_passphrase = checked;
                                            }
                                            cx.notify();
                                        })),
                                ),
                        );
                        content = if form.save_passphrase {
                            content.child(Self::secret_field(
                                rust_i18n::t!("form.passphrase").to_string(),
                                form.passphrase.clone(),
                                p,
                            ))
                        } else {
                            content.child(Self::credential_deferred_hint(p))
                        };
                    }
                    AuthChoice::Agent => {}
                }
            }
            ConnectionChoice::Serial => {
                content = content.child(render_serial_fields(form, cx, p));
            }
        }

        content = content.child(
            v_flex()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(rust_i18n::t!("form.startup_commands").to_string()),
                )
                .child(Textarea::new(&form.startup_commands).h(px(88.0)).w_full())
                .child(
                    div()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(rust_i18n::t!("form.startup_hint").to_string()),
                ),
        );

        let editing = form.base.is_some();
        let confirm_delete = form.confirm_delete;
        let mut footer = h_flex().gap_2().items_center().justify_end().w_full();
        if editing {
            if confirm_delete {
                footer = footer.child(
                    Button::new("confirm-delete")
                        .danger()
                        .small()
                        .icon(IconName::Trash)
                        .label(rust_i18n::t!("common.confirm_delete").to_string())
                        .on_click(cx.listener(|this, _, _, cx| this.delete_session_form(cx))),
                );
            } else {
                footer = footer.child(
                    Button::new("delete-session")
                        .ghost()
                        .small()
                        .icon(IconName::Trash)
                        .label(rust_i18n::t!("common.delete").to_string())
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(form) = this.form.as_mut() {
                                form.confirm_delete = true;
                            }
                            cx.notify();
                        })),
                );
            }
        }
        footer = footer
            .child(
                Button::new("cancel-session")
                    .ghost()
                    .label(rust_i18n::t!("common.cancel").to_string())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.form = None;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("save-session")
                    .primary()
                    .icon(IconName::Check)
                    .label(rust_i18n::t!("common.save").to_string())
                    .on_click(cx.listener(|this, _, _, cx| this.save_session_form(cx))),
            );
        // 校验错误与按钮同处外壳：内容区会滚动，报错不能被滚出视野，
        // 否则保存失败看起来就像"点了没反应"。
        let mut footer_block = v_flex().gap_2().items_end();
        if let Some(error) = &form.error {
            footer_block =
                footer_block.child(div().text_sm().text_color(p.danger).child(error.clone()));
        }
        footer_block = footer_block.child(footer);

        let dialog = Self::dialog_frame(
            "session-form-modal",
            title,
            None,
            content,
            Some(footer_block),
            px(620.0),
            p,
        );
        Self::overlay(dialog, p)
    }

    /// 用系统文件选择器挑选私钥，起始目录 `~/.ssh`。
    ///
    /// 不设扩展名过滤：OpenSSH 私钥通常没有扩展名，过滤反而挡掉常见文件。
    /// 输入框仍可手输，路径在保存时统一展开 `~`。
    pub(crate) fn pick_private_key_path(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = self.form.as_mut() else {
            return;
        };
        let title = rust_i18n::t!("form.pick_key").to_string();
        let Some(path) = rfd::FileDialog::new()
            .set_title(&title)
            .set_directory(crate::keys::default_ssh_dir())
            .pick_file()
        else {
            return;
        };
        let text = path.to_string_lossy().to_string();
        form.key_path
            .update(cx, |state, cx| state.set_value(&text, window, cx));
        cx.notify();
    }

    /// "本次不保存凭据"的说明行：告诉用户密码会在连接时询问，而不是漏填。
    fn credential_deferred_hint(p: &Palette) -> AnyElement {
        div()
            .text_xs()
            .text_color(p.muted_foreground)
            .child(rust_i18n::t!("form.credential_deferred").to_string())
            .into_any_element()
    }
}
