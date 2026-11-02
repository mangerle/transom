//! 容器引擎连接表单的图标与环境色选择。

use super::*;

/// 外观色板色块尺寸；与会话表单保持一致。
const APPEARANCE_SWATCH_SIZE: f32 = 18.0;

impl AppShell {
    pub(super) fn render_engine_form_appearance_card(
        &self,
        app: &Entity<AppShell>,
        form: &ContainerFormState,
        p: &Palette,
    ) -> AnyElement {
        let default_icon = session_icon(form.icon).unwrap_or(IconName::Container);

        // 图标选择行
        let icon_row = h_flex()
            .gap_1()
            .child(
                Button::new("container-form-icon-auto")
                    .small()
                    .icon(default_icon)
                    .selected(form.icon == SessionIcon::Auto)
                    .tooltip(rust_i18n::t!("form.icon_auto").to_string())
                    .on_click(crate::dialog_host::on_event(
                        app,
                        |this, _: &ClickEvent, _, cx| {
                            if let Some(form) = this.state.containers.form.as_mut() {
                                form.icon = SessionIcon::Auto;
                            }
                            cx.notify();
                        },
                    )),
            )
            .children(SESSION_ICON_CHOICES.into_iter().map(|choice| {
                let Some(name) = session_icon(choice) else {
                    return div().into_any_element();
                };
                Button::new(ElementId::Name(
                    format!("container-form-icon-{choice:?}").into(),
                ))
                .small()
                .icon(name)
                .selected(form.icon == choice)
                .on_click(crate::dialog_host::on_event(
                    app,
                    move |this, _: &ClickEvent, _, cx| {
                        if let Some(form) = this.state.containers.form.as_mut() {
                            form.icon = choice;
                        }
                        cx.notify();
                    },
                ))
                .into_any_element()
            }));

        // 颜色选择行
        let color_row = h_flex()
            .gap_2()
            .child(
                div()
                    .id("container-form-color-none")
                    .size(px(APPEARANCE_SWATCH_SIZE))
                    .rounded_full()
                    .border_1()
                    .border_color(if form.color == SessionColor::None {
                        p.primary
                    } else {
                        p.border
                    })
                    .bg(p.background)
                    .cursor_pointer()
                    .tooltip(move |_window, cx| {
                        Tooltip::new(rust_i18n::t!("form.color_none").to_string())
                            .build(_window, cx)
                    })
                    .on_click(crate::dialog_host::on_event(
                        app,
                        |this, _: &ClickEvent, _, cx| {
                            if let Some(form) = this.state.containers.form.as_mut() {
                                form.color = SessionColor::None;
                            }
                            cx.notify();
                        },
                    )),
            )
            .children(SESSION_COLOR_CHOICES.into_iter().map(|choice| {
                let Some(value) = session_color(choice) else {
                    return div().into_any_element();
                };
                div()
                    .id(ElementId::Name(
                        format!("container-form-color-{choice:?}").into(),
                    ))
                    .size(px(APPEARANCE_SWATCH_SIZE))
                    .rounded_full()
                    .bg(value)
                    .border_1()
                    .border_color(if form.color == choice {
                        p.foreground
                    } else {
                        p.border
                    })
                    .cursor_pointer()
                    .on_click(crate::dialog_host::on_event(
                        app,
                        move |this, _: &ClickEvent, _, cx| {
                            if let Some(form) = this.state.containers.form.as_mut() {
                                form.color = choice;
                            }
                            cx.notify();
                        },
                    ))
                    .into_any_element()
            }));

        v_flex()
            .gap_2()
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("form.icon").to_string()),
            )
            .child(icon_row)
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("form.color").to_string()),
            )
            .child(color_row)
            .into_any_element()
    }

    // -------------------------------------------------------------------------
    // 连接类型卡：引擎种类 + 端点分组（本地 / 远端）
    // -------------------------------------------------------------------------
}
