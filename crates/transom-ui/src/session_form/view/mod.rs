//! 会话表单弹窗主体渲染。
//!
//! 渲染归实体：按钮回调统一经 `cx.listener` 回到 [`SessionFormView`]，
//! 需要目录或串口数据时通过 [`SessionFormEvent`] 上交意图，实体不在渲染中
//! 反查外壳。

use super::entity::SessionFormView;
use super::local::render_local_fields;
use super::serial::render_serial_fields;
use super::ssh::{render_ssh_basic_fields, render_ssh_network_fields};
use super::telnet::{render_telnet_basic_fields, render_telnet_network_fields};
use super::types::{ConnectionChoice, SessionForm, SessionFormSection};
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
mod forward;
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

    /// 渲染当前选中的右侧分组内容。
    fn render_section_content(
        &self,
        form: &SessionForm,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        match form.section {
            SessionFormSection::Basic => {
                let header_row = self.render_session_basic_header_row(form, p, cx);
                let protocol_fields = match form.connection {
                    ConnectionChoice::Ssh | ConnectionChoice::Mosh => {
                        render_ssh_basic_fields(form, cx, p)
                    }
                    ConnectionChoice::Local => render_local_fields(form, cx, p),
                    ConnectionChoice::Serial => render_serial_fields(form, cx, p),
                    ConnectionChoice::Telnet => render_telnet_basic_fields(form, cx, p),
                };
                v_flex()
                    .gap_3()
                    .child(header_row)
                    .child(protocol_fields)
                    .into_any_element()
            }
            SessionFormSection::Network => match form.connection {
                ConnectionChoice::Ssh | ConnectionChoice::Mosh => {
                    render_ssh_network_fields(form, cx, p)
                }
                ConnectionChoice::Telnet => render_telnet_network_fields(form, cx, p),
                ConnectionChoice::Local | ConnectionChoice::Serial => div()
                    .p_3()
                    .rounded(p.radius.md)
                    .bg(p.muted)
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("form.network_not_applicable").to_string())
                    .into_any_element(),
            },
            SessionFormSection::Terminal => self.render_session_terminal_section(form, p, cx),
            SessionFormSection::Forward => self.render_session_forward_section(form, p, cx),
            SessionFormSection::Startup => self.render_session_startup_section(form, p),
        }
    }

    /// 弹窗主体：顶部全宽协议栏 + 下方左右双栏（左侧分组导航、右侧对应表单区）。
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

        let protocol_bar = self.render_session_connection_type_bar(form, p, cx);
        let sidebar = self.render_session_sidebar(form, p, cx);
        let section_title = rust_i18n::t!(form.section.title_key()).to_string();
        let section_body = self.render_section_content(form, cx, p);

        let right_pane = v_flex()
            .id(ElementId::Name("session-form-body".into()))
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_y_scroll()
            .pr_1()
            .gap_3()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(p.foreground)
                    .child(section_title),
            )
            .child(section_body);

        let content = v_flex()
            .w_full()
            .gap_4()
            .child(protocol_bar)
            .child(
                h_flex()
                    .w_full()
                    .h(rems(27.5))
                    .gap_4()
                    .items_start()
                    .child(sidebar)
                    .child(right_pane),
            )
            .into_any_element();

        let footer = self.render_footer(form, cx, p);

        crate::widgets::dialog_body("session-form-modal", title, None, content, Some(footer))
    }
}
