//! 会话表单中的串口连接配置渲染。

use crate::app::AppShell;
use crate::palette::Palette;
use crate::session_form::SessionForm;
use gpui_kit::assets::IconName;
use gpui_kit::base::{Selectable, h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::Button;
use gpui_kit::*;
use transom_core::{SerialDataBits, SerialFlowControl, SerialParity, SerialStopBits};

pub(crate) fn render_serial_fields(
    form: &SessionForm,
    cx: &mut Context<AppShell>,
    p: &Palette,
) -> AnyElement {
    let mut fields = v_flex().gap_3();
    let current_port = form.serial_port.read(cx).value().trim().to_string();
    let mut port_chips = h_flex().gap_1().flex_wrap();
    for (index, descriptor) in form.serial_ports.iter().enumerate() {
        let port_name = descriptor.port_name.clone();
        let label = descriptor
            .description
            .as_ref()
            .map(|description| format!("{port_name} · {description}"))
            .unwrap_or_else(|| port_name.clone());
        port_chips = port_chips.child(
            Button::new(ElementId::Name(format!("serial-port-{index}").into()))
                .xsmall()
                .label(label)
                .selected(current_port == port_name)
                .on_click(cx.listener(move |this, _, window, cx| {
                    if let Some(form) = this.form.as_mut() {
                        let port_name = port_name.clone();
                        form.serial_port.update(cx, |state, cx| {
                            state.set_value(&port_name, window, cx);
                        });
                    }
                    cx.notify();
                })),
        );
    }
    let ports_hint = form.serial_ports_error.clone().unwrap_or_else(|| {
        if form.serial_ports.is_empty() {
            rust_i18n::t!("form.serial_no_ports").to_string()
        } else {
            rust_i18n::t!("form.serial_detected").to_string()
        }
    });
    let data_bits_buttons = h_flex()
        .gap_1()
        .children(SerialDataBits::ALL.into_iter().map(|bits| {
            Button::new(ElementId::Name(
                format!("serial-bits-{}", bits.bits()).into(),
            ))
            .small()
            .label(bits.bits().to_string())
            .selected(form.serial_data_bits == bits)
            .on_click(cx.listener(move |this, _, _, cx| {
                if let Some(form) = this.form.as_mut() {
                    form.serial_data_bits = bits;
                }
                cx.notify();
            }))
        }));
    let parity_buttons = h_flex().gap_1().children(
        [
            (SerialParity::None, "form.parity_none"),
            (SerialParity::Odd, "form.parity_odd"),
            (SerialParity::Even, "form.parity_even"),
        ]
        .into_iter()
        .map(|(parity, label)| {
            Button::new(ElementId::Name(format!("serial-parity-{label}").into()))
                .small()
                .label(rust_i18n::t!(label).to_string())
                .selected(form.serial_parity == parity)
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(form) = this.form.as_mut() {
                        form.serial_parity = parity;
                    }
                    cx.notify();
                }))
        }),
    );
    let stop_bits_buttons = h_flex().gap_1().children(
        [
            (SerialStopBits::One, "form.stop_bits_one"),
            (SerialStopBits::Two, "form.stop_bits_two"),
        ]
        .into_iter()
        .map(|(stop_bits, label)| {
            Button::new(ElementId::Name(format!("serial-stop-{label}").into()))
                .small()
                .label(rust_i18n::t!(label).to_string())
                .selected(form.serial_stop_bits == stop_bits)
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(form) = this.form.as_mut() {
                        form.serial_stop_bits = stop_bits;
                    }
                    cx.notify();
                }))
        }),
    );
    let flow_buttons = h_flex().gap_1().children(
        [
            (SerialFlowControl::None, "form.flow_none"),
            (SerialFlowControl::Software, "form.flow_software"),
            (SerialFlowControl::Hardware, "form.flow_hardware"),
        ]
        .into_iter()
        .map(|(flow, label)| {
            Button::new(ElementId::Name(format!("serial-flow-{label}").into()))
                .small()
                .label(rust_i18n::t!(label).to_string())
                .selected(form.serial_flow_control == flow)
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(form) = this.form.as_mut() {
                        form.serial_flow_control = flow;
                    }
                    cx.notify();
                }))
        }),
    );

    fields = fields
        .child(
            v_flex()
                .gap_2()
                .child(
                    h_flex()
                        .gap_2()
                        .items_end()
                        .child(div().flex_1().child(AppShell::form_field(
                            rust_i18n::t!("form.serial_port").to_string(),
                            form.serial_port.clone(),
                            p,
                        )))
                        .child(
                            Button::new("serial-refresh")
                                .small()
                                .icon(IconName::RefreshCw)
                                .label(rust_i18n::t!("form.serial_refresh").to_string())
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.refresh_serial_ports(window, cx);
                                })),
                        ),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(if form.serial_ports_error.is_some() {
                            p.danger
                        } else {
                            p.muted_foreground
                        })
                        .child(ports_hint),
                )
                .child(port_chips),
        )
        .child(
            h_flex()
                .gap_3()
                .items_end()
                .child(div().w(px(160.0)).child(AppShell::form_field(
                    rust_i18n::t!("form.serial_baud").to_string(),
                    form.serial_baud.clone(),
                    p,
                )))
                .child(div().flex_1().child(serial_option_row(
                    rust_i18n::t!("form.serial_data_bits").to_string(),
                    data_bits_buttons,
                    p,
                ))),
        )
        .child(serial_option_row(
            rust_i18n::t!("form.serial_parity").to_string(),
            parity_buttons,
            p,
        ))
        .child(serial_option_row(
            rust_i18n::t!("form.serial_stop_bits").to_string(),
            stop_bits_buttons,
            p,
        ))
        .child(serial_option_row(
            rust_i18n::t!("form.serial_flow_control").to_string(),
            flow_buttons,
            p,
        ));
    fields.into_any_element()
}

fn serial_option_row(label: String, choices: impl IntoElement, p: &Palette) -> AnyElement {
    h_flex()
        .justify_between()
        .items_center()
        .py_1()
        .child(div().text_sm().text_color(p.foreground).child(label))
        .child(choices)
        .into_any_element()
}
