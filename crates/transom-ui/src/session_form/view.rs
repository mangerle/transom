//! 会话表单弹窗主体外壳渲染与文件选择逻辑。

use super::appearance::default_icon_for;
use super::serial::render_serial_fields;
use super::ssh::render_ssh_fields;
use super::telnet::render_telnet_fields;
use super::types::ConnectionChoice;
use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{Selectable, h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Textarea;
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::*;
use transom_core::BlockViewOverride;

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

        // 内容区自带有限高度的滚动区：标题、错误提示与操作按钮由 `dialog_frame`
        // 固定在外壳上，字段再多也不会把它们移出视野。
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
                    .child(
                        div().flex_1().child(
                            v_flex()
                                .gap_1()
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(p.muted_foreground)
                                        .child(rust_i18n::t!("form.folder").to_string()),
                                )
                                .child(
                                    h_flex()
                                        .gap_1p5()
                                        .items_center()
                                        .child(div().flex_1().child(form.folder_path.clone()))
                                        .child(
                                            Button::new("folder-select-dropdown")
                                                .outline()
                                                .icon(IconName::Folder)
                                                .dropdown_menu({
                                                    let options = form.folder_options.clone();
                                                    let app_entity = cx.entity().clone();
                                                    move |menu, _window, _cx| {
                                                        let app = app_entity.clone();
                                                        let mut menu = menu.item(
                                                            PopupMenuItem::new(
                                                                rust_i18n::t!("folder.root_level")
                                                                    .to_string(),
                                                            )
                                                            .on_click({
                                                                let app = app.clone();
                                                                move |_, window, cx| {
                                                                    app.update(cx, |this, cx| {
                                                                        this.clear_form_folder(
                                                                            window, cx,
                                                                        );
                                                                    });
                                                                }
                                                            }),
                                                        );
                                                        if !options.is_empty() {
                                                            menu = menu
                                                                .item(PopupMenuItem::separator());
                                                            for opt in &options {
                                                                let path = opt.clone();
                                                                let app = app.clone();
                                                                menu = menu.item(
                                                                    PopupMenuItem::new(path.clone())
                                                                        .icon(IconName::Folder)
                                                                        .on_click(
                                                                            move |_, window, cx| {
                                                                                app.update(
                                                                                    cx,
                                                                                    |this, cx| {
                                                                                        this.select_form_folder(
                                                                                            &path,
                                                                                            window,
                                                                                            cx,
                                                                                        );
                                                                                    },
                                                                                );
                                                                            },
                                                                        ),
                                                                );
                                                            }
                                                        }
                                                        menu
                                                    }
                                                }),
                                        ),
                                ),
                        ),
                    ),
            )
            .child(Self::render_session_appearance(
                default_icon_for(form.connection),
                form.icon,
                form.color,
                p,
                cx,
            ))
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
                            )
                            .child(
                                Button::new("conn-telnet")
                                    .small()
                                    .label("Telnet")
                                    .selected(form.connection == ConnectionChoice::Telnet)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if let Some(form) = this.form.as_mut() {
                                            form.connection = ConnectionChoice::Telnet;
                                        }
                                        cx.notify();
                                    })),
                            ),
                    ),
            );

        match form.connection {
            ConnectionChoice::Ssh => {
                content = content.child(render_ssh_fields(form, cx, p));
            }
            ConnectionChoice::Serial => {
                content = content.child(render_serial_fields(form, cx, p));
            }
            ConnectionChoice::Telnet => {
                content = content.child(render_telnet_fields(form, cx, p));
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

        // 语义块模式：跟随全局/强制开启/强制关闭，所有连接类型通用。
        let block_choice = form.block_view;
        content = content.child(
            v_flex()
                .gap_2()
                .child(
                    div()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(rust_i18n::t!("form.block_view").to_string()),
                )
                .child(
                    h_flex().gap_2().children(
                        [
                            (
                                "block-follow-global",
                                "form.block_view_follow",
                                BlockViewOverride::FollowGlobal,
                            ),
                            (
                                "block-enabled",
                                "form.block_view_enabled",
                                BlockViewOverride::Enabled,
                            ),
                            (
                                "block-disabled",
                                "form.block_view_disabled",
                                BlockViewOverride::Disabled,
                            ),
                        ]
                        .into_iter()
                        .map(|(id, key, value)| {
                            Button::new(id)
                                .small()
                                .label(rust_i18n::t!(key).to_string())
                                .selected(block_choice == value)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(form) = this.form.as_mut() {
                                        form.block_view = value;
                                    }
                                    cx.notify();
                                }))
                        }),
                    ),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(rust_i18n::t!("form.block_view_hint").to_string()),
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

    /// 选择 CA 签发的用户证书文件（OpenSSH 惯例是 `私钥名-cert.pub`）。
    pub(crate) fn pick_certificate_path(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = self.form.as_mut() else {
            return;
        };
        let title = rust_i18n::t!("form.pick_cert").to_string();
        let Some(path) = rfd::FileDialog::new()
            .set_title(&title)
            .set_directory(crate::keys::default_ssh_dir())
            .add_filter("OpenSSH 证书", &["pub"])
            .pick_file()
        else {
            return;
        };
        let text = path.to_string_lossy().to_string();
        form.cert_path
            .update(cx, |state, cx| state.set_value(&text, window, cx));
        cx.notify();
    }
}
