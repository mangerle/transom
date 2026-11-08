//! 会话表单的连接类型卡片与 Mosh 专属字段。

use super::*;

impl SessionFormView {
    pub(super) fn render_session_connection_type_card(
        &self,
        form: &SessionForm,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
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
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.set_connection(ConnectionChoice::Ssh, cx);
                            })),
                    )
                    .child(
                        Button::new("conn-serial")
                            .small()
                            .label(rust_i18n::t!("form.conn_serial").to_string())
                            .selected(form.connection == ConnectionChoice::Serial)
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.set_connection(ConnectionChoice::Serial, cx);
                            })),
                    )
                    .child(
                        Button::new("conn-telnet")
                            .small()
                            .label("Telnet")
                            .selected(form.connection == ConnectionChoice::Telnet)
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.set_connection(ConnectionChoice::Telnet, cx);
                            })),
                    )
                    .child(
                        Button::new("conn-mosh")
                            .small()
                            .label("Mosh")
                            .selected(form.connection == ConnectionChoice::Mosh)
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.set_connection(ConnectionChoice::Mosh, cx);
                            })),
                    ),
            )
            .into_any_element()
    }

    pub(super) fn render_mosh_specific_card(&self, form: &SessionForm, p: &Palette) -> AnyElement {
        v_flex()
            .gap_2()
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("form.mosh.group").to_string()),
            )
            .child(
                h_flex()
                    .gap_3()
                    .child(div().flex_1().child(crate::widgets::form_field(
                        rust_i18n::t!("form.mosh.server_path").to_string(),
                        form.mosh_server_path.clone(),
                        p,
                    )))
                    .child(div().flex_1().child(crate::widgets::form_field(
                        rust_i18n::t!("form.mosh.server_port").to_string(),
                        form.mosh_server_port.clone(),
                        p,
                    ))),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("form.mosh.hint").to_string()),
            )
            .into_any_element()
    }
}
