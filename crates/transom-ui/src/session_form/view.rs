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
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, Textarea};
use gpui_kit::component::popover::Popover;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;
use transom_core::BlockViewOverride;

mod basic;
mod connection;
mod files;
mod terminal;

impl AppShell {
    pub(crate) fn render_session_form_body(
        &self,
        app: &Entity<AppShell>,
        _window: &mut Window,
        cx: &App,
        p: &Palette,
    ) -> AnyElement {
        let Some(form) = self.modals.form.as_ref() else {
            return div().into_any_element();
        };
        let title = if form.base.is_some() {
            rust_i18n::t!("form.edit_title").to_string()
        } else {
            rust_i18n::t!("form.new_title").to_string()
        };

        let basic_card = Self::render_session_basic_card(form, app, p, cx);
        let connection_type_card = Self::render_session_connection_type_card(form, app, cx, p);

        let mut content = v_flex()
            .id(ElementId::Name("session-form-body".into()))
            .max_h(px(460.0))
            .overflow_y_scroll()
            .gap_3()
            .child(basic_card)
            .child(connection_type_card);

        match form.connection {
            ConnectionChoice::Ssh => {
                content = content.child(render_ssh_fields(form, app, cx, p));
            }
            ConnectionChoice::Mosh => {
                content = content
                    .child(render_ssh_fields(form, app, cx, p))
                    .child(Self::render_mosh_specific_card(form, p));
            }
            ConnectionChoice::Serial => {
                content = content.child(render_serial_fields(form, app, cx, p));
            }
            ConnectionChoice::Telnet => {
                content = content.child(render_telnet_fields(form, app, cx, p));
            }
        }

        content = content.child(Self::render_session_terminal_attributes_card(
            form, app, p, cx,
        ));

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
                        .on_click(crate::dialog_host::on_event(
                            app,
                            |this, _: &ClickEvent, _, cx| this.delete_session_form(cx),
                        )),
                );
            } else {
                footer = footer.child(
                    Button::new("delete-session")
                        .ghost()
                        .small()
                        .icon(IconName::Trash)
                        .label(rust_i18n::t!("common.delete").to_string())
                        .on_click(crate::dialog_host::on_event(
                            app,
                            |this, _: &ClickEvent, _, cx| {
                                if let Some(form) = this.modals.form.as_mut() {
                                    form.confirm_delete = true;
                                }
                                cx.notify();
                            },
                        )),
                );
            }
        }
        footer = footer
            .child(
                Button::new("cancel-session")
                    .ghost()
                    .label(rust_i18n::t!("common.cancel").to_string())
                    .on_click(crate::dialog_host::on_event(
                        app,
                        |this, _: &ClickEvent, _, cx| {
                            this.modals.form = None;
                            cx.notify();
                        },
                    )),
            )
            .child(
                Button::new("save-session")
                    .primary()
                    .icon(IconName::Check)
                    .label(rust_i18n::t!("common.save").to_string())
                    .on_click(crate::dialog_host::on_event(
                        app,
                        |this, _: &ClickEvent, _, cx| this.save_session_form(cx),
                    )),
            );

        let mut footer_block = v_flex().gap_2().items_end();
        if let Some(error) = &form.error {
            footer_block =
                footer_block.child(div().text_sm().text_color(p.danger).child(error.clone()));
        }
        footer_block = footer_block.child(footer);

        crate::widgets::dialog_body(
            "session-form-modal",
            title,
            None,
            content,
            Some(footer_block),
        )
    }
}
