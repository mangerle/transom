//! 会话表单弹窗主体渲染。
//!
//! 渲染归实体：按钮回调统一经 `cx.listener` 回到 [`SessionFormView`]，
//! 需要目录或串口数据时通过 [`SessionFormEvent`] 上交意图，实体不在渲染中
//! 反查外壳。

use super::entity::SessionFormView;
use super::local::render_local_fields;
use super::serial::render_serial_fields;
use super::ssh::render_ssh_fields;
use super::telnet::render_telnet_fields;
use super::types::{ConnectionChoice, SessionForm};
use crate::palette::Palette;
use gpui_kit::base::{Selectable, h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, Textarea};
use gpui_kit::component::popover::Popover;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::*;
use transom_core::BlockViewOverride;
use transom_ui_icons::AppIcon;
use transom_ui_icons::AppIconElement;
use transom_ui_icons::UiIcon;

mod basic;
mod connection;
mod files;
mod terminal;

impl SessionFormView {
    /// 表单页脚：删除/确认删除、取消与保存，错误提示在页脚上方。
    fn render_footer(&self, form: &SessionForm, cx: &mut Context<Self>, p: &Palette) -> AnyElement {
        let editing = form.base.is_some();
        let confirm_delete = form.confirm_delete;
        let mut footer = h_flex().gap_2().items_center().justify_end().w_full();
        if editing {
            if confirm_delete {
                footer = footer.child(
                    Button::new("confirm-delete")
                        .danger()
                        .small()
                        .child(AppIconElement::new(AppIcon::Ui(UiIcon::DELETE)).small())
                        .label(rust_i18n::t!("common.confirm_delete").to_string())
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.delete(cx))),
                );
            } else {
                footer = footer.child(
                    Button::new("delete-session")
                        .ghost()
                        .small()
                        .child(AppIconElement::new(AppIcon::Ui(UiIcon::DELETE)).small())
                        .label(rust_i18n::t!("common.delete").to_string())
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.edit(cx, |form| form.confirm_delete = true);
                        })),
                );
            }
        }
        footer = footer
            .child(crate::widgets::dialog_cancel_button(
                "cancel-session",
                rust_i18n::t!("common.cancel").to_string(),
                cx.listener(|this, _: &ClickEvent, _, cx| this.close(cx)),
            ))
            .child(crate::widgets::dialog_confirm_button(
                "save-session",
                rust_i18n::t!("common.save").to_string(),
                Some(AppIcon::Ui(UiIcon::CHECK)),
                crate::widgets::DialogConfirmKind::Primary,
                cx.listener(|this, _: &ClickEvent, _, cx| this.save(cx)),
            ));

        let mut footer_block = v_flex().gap_2().items_end();
        if let Some(error) = &form.error {
            footer_block =
                footer_block.child(div().text_sm().text_color(p.danger).child(error.clone()));
        }
        footer_block = footer_block.child(footer);

        footer_block.into_any_element()
    }

    /// 弹窗主体：基本信息、连接类型与按连接类型展开的字段。
    pub(crate) fn render_body(
        &self,
        _window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let Some(form) = self.state() else {
            return div().into_any_element();
        };
        let title = if form.base.is_some() {
            rust_i18n::t!("form.edit_title").to_string()
        } else {
            rust_i18n::t!("form.new_title").to_string()
        };

        let basic_card = self.render_session_basic_card(form, p, cx);
        let connection_type_card = self.render_session_connection_type_card(form, p, cx);

        let mut content = v_flex()
            .id(ElementId::Name("session-form-body".into()))
            .max_h(px(460.0))
            .overflow_y_scroll()
            .gap_3()
            .child(basic_card)
            .child(connection_type_card);

        match form.connection {
            ConnectionChoice::Ssh => {
                content = content.child(render_ssh_fields(form, cx, p));
            }
            ConnectionChoice::Local => {
                content = content.child(render_local_fields(form, cx, p));
            }
            ConnectionChoice::Mosh => {
                content = content
                    .child(render_ssh_fields(form, cx, p))
                    .child(self.render_mosh_specific_card(form, p));
            }
            ConnectionChoice::Serial => {
                content = content.child(render_serial_fields(form, cx, p));
            }
            ConnectionChoice::Telnet => {
                content = content.child(render_telnet_fields(form, cx, p));
            }
        }

        content = content.child(self.render_session_terminal_attributes_card(form, p, cx));

        let footer = self.render_footer(form, cx, p);

        crate::widgets::dialog_body("session-form-modal", title, None, content, Some(footer))
    }
}
