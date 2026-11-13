//! 会话表单的“终端设置”与“登录后命令”分组。

use super::*;

impl SessionFormView {
    /// 渲染“终端设置”分组（语义命令块覆盖开关等）。
    pub(super) fn render_session_terminal_section(
        &self,
        form: &SessionForm,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let block_choice = form.block_view;

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
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.edit(cx, |form| form.block_view = value);
                            }))
                    }),
                ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("form.block_view_hint").to_string()),
            )
            .into_any_element()
    }

    /// 渲染“登录后命令”分组（多行启动命令输入框与提示）。
    pub(super) fn render_session_startup_section(
        &self,
        form: &SessionForm,
        p: &Palette,
    ) -> AnyElement {
        v_flex()
            .gap_2()
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("form.startup_commands").to_string()),
            )
            .child(Textarea::new(&form.startup_commands).h(rems(9.5)).w_full())
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("form.startup_hint").to_string()),
            )
            .into_any_element()
    }
}
