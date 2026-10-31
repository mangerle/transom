//! 会话表单的终端属性卡片。

use super::*;

impl AppShell {
    pub(super) fn render_session_terminal_attributes_card(
        form: &crate::session_form::types::SessionForm,
        app: &Entity<AppShell>,
        p: &Palette,
        _cx: &App,
    ) -> AnyElement {
        let block_choice = form.block_view;

        v_flex()
            .gap_3()
            .child(
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
            )
            .child(
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
                                    .on_click(crate::dialog_host::on_event(
                                        app,
                                        move |this, _: &ClickEvent, _, cx| {
                                            if let Some(form) = this.state.modals.form.as_mut() {
                                                form.block_view = value;
                                            }
                                            cx.notify();
                                        },
                                    ))
                            }),
                        ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child(rust_i18n::t!("form.block_view_hint").to_string()),
                    ),
            )
            .into_any_element()
    }
}
