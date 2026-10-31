//! 会话表单的连接类型卡片与 Mosh 专属字段。

use super::*;

impl AppShell {
    pub(super) fn render_session_connection_type_card(
        form: &crate::session_form::types::SessionForm,
        app: &Entity<AppShell>,
        _cx: &App,
        p: &Palette,
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
                            .on_click(crate::dialog_host::on_event(
                                app,
                                |this, _: &ClickEvent, _, cx| {
                                    if let Some(form) = this.modals.form.as_mut() {
                                        form.connection = ConnectionChoice::Ssh;
                                    }
                                    cx.notify();
                                },
                            )),
                    )
                    .child(
                        Button::new("conn-serial")
                            .small()
                            .label(rust_i18n::t!("form.conn_serial").to_string())
                            .selected(form.connection == ConnectionChoice::Serial)
                            .on_click(crate::dialog_host::on_event(
                                app,
                                |this, _: &ClickEvent, window, cx| {
                                    let needs_ports = if let Some(form) = this.modals.form.as_mut()
                                    {
                                        form.connection = ConnectionChoice::Serial;
                                        form.serial_ports.is_empty()
                                    } else {
                                        false
                                    };
                                    if needs_ports {
                                        this.refresh_serial_ports(window, cx);
                                    }
                                    cx.notify();
                                },
                            )),
                    )
                    .child(
                        Button::new("conn-telnet")
                            .small()
                            .label("Telnet")
                            .selected(form.connection == ConnectionChoice::Telnet)
                            .on_click(crate::dialog_host::on_event(
                                app,
                                |this, _: &ClickEvent, _, cx| {
                                    if let Some(form) = this.modals.form.as_mut() {
                                        form.connection = ConnectionChoice::Telnet;
                                    }
                                    cx.notify();
                                },
                            )),
                    )
                    .child(
                        Button::new("conn-mosh")
                            .small()
                            .label("Mosh")
                            .selected(form.connection == ConnectionChoice::Mosh)
                            .on_click(crate::dialog_host::on_event(
                                app,
                                |this, _: &ClickEvent, _, cx| {
                                    if let Some(form) = this.modals.form.as_mut() {
                                        form.connection = ConnectionChoice::Mosh;
                                    }
                                    cx.notify();
                                },
                            )),
                    ),
            )
            .into_any_element()
    }

    pub(super) fn render_mosh_specific_card(
        form: &crate::session_form::types::SessionForm,
        p: &Palette,
    ) -> AnyElement {
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
