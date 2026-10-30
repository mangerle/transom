//! 偏好设置中心之终端字体、光标与按键映射配置页渲染。

use super::FontFieldSpec;
use crate::app::AppShell;
use crate::divider::{DividerEdge, DividerStyled};
use crate::palette::Palette;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::input::Textarea;
use gpui_kit::component::switch::Switch;
use gpui_kit::*;
use transom_core::TerminalCursorShape;

impl AppShell {
    /// 终端字体组：终端字体族、CJK 字体、字号与行高。
    pub(crate) fn render_settings_terminal_font(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let stepper_size = Self::setting_stepper(&self.modals.settings.font_size, px(110.0));
        let stepper_height = Self::setting_stepper(&self.modals.settings.line_height, px(110.0));
        let metrics_row = h_flex()
            .w_full()
            .gap_6()
            .py_2()
            .border_color(p.divider(p.border))
            .divider_edge(DividerEdge::Bottom, p.divider_width)
            .child(
                h_flex()
                    .flex_1()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .text_sm()
                            .child(rust_i18n::t!("settings.font_size").to_string()),
                    )
                    .child(stepper_size),
            )
            .child(
                h_flex()
                    .flex_1()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .text_sm()
                            .child(rust_i18n::t!("settings.line_height").to_string()),
                    )
                    .child(stepper_height),
            );

        v_flex()
            .gap_2()
            .child(Self::setting_font_field(
                FontFieldSpec {
                    id: "terminal-font-family",
                    label: rust_i18n::t!("settings.font_family").to_string(),
                    select: self.modals.settings.font_family.clone(),
                    note_key: None,
                },
                p,
                cx,
            ))
            .child(Self::setting_font_field(
                FontFieldSpec {
                    id: "terminal-cjk-font",
                    label: rust_i18n::t!("settings.cjk_font").to_string(),
                    select: self.modals.settings.cjk_font.clone(),
                    note_key: Some("settings.cjk_font_note"),
                },
                p,
                cx,
            ))
            .child(metrics_row)
            .into_any_element()
    }

    /// 光标与滚动组：光标形态、闪烁、粗体亮色、输入滚动与回滚行数。
    pub(crate) fn render_settings_cursor(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let cursor_shape = self.settings.terminal_profile.cursor_shape;
        let cursor_options = vec![
            (
                TerminalCursorShape::Block,
                rust_i18n::t!("settings.cursor_block").to_string(),
            ),
            (
                TerminalCursorShape::Bar,
                rust_i18n::t!("settings.cursor_bar").to_string(),
            ),
            (
                TerminalCursorShape::Underline,
                rust_i18n::t!("settings.cursor_underline").to_string(),
            ),
        ];
        let cursor_dropdown = Self::setting_dropdown(
            "dropdown-cursor-shape",
            cursor_shape,
            cursor_options,
            |this, shape, cx| {
                this.settings.terminal_profile.cursor_shape = shape;
                cx.notify();
            },
            cx,
        );

        v_flex()
            .gap_2()
            .child(Self::setting_row(
                "settings.cursor_shape",
                Some("settings.cursor_shape_desc"),
                cursor_dropdown,
                p,
            ))
            .child(Self::setting_row(
                "settings.cursor_blink",
                Some("settings.cursor_blink_desc"),
                Switch::new("toggle-cursor-blink")
                    .small()
                    .checked(self.settings.terminal_profile.cursor_blink)
                    .on_click(cx.listener(|this, &checked, _, cx| {
                        this.settings.terminal_profile.cursor_blink = checked;
                        cx.notify();
                    }))
                    .into_any_element(),
                p,
            ))
            .child(Self::setting_row(
                "settings.bold_is_bright",
                Some("settings.bold_is_bright_desc"),
                Switch::new("toggle-bold-is-bright")
                    .small()
                    .checked(self.settings.terminal_profile.bold_is_bright)
                    .on_click(cx.listener(|this, &checked, _, cx| {
                        this.settings.terminal_profile.bold_is_bright = checked;
                        cx.notify();
                    }))
                    .into_any_element(),
                p,
            ))
            .child(Self::setting_row(
                "settings.alternate_scroll",
                Some("settings.alternate_scroll_desc"),
                Switch::new("toggle-alternate-scroll")
                    .small()
                    .checked(self.settings.terminal_profile.alternate_scroll)
                    .on_click(cx.listener(|this, &checked, _, cx| {
                        this.settings.terminal_profile.alternate_scroll = checked;
                        cx.notify();
                    }))
                    .into_any_element(),
                p,
            ))
            .child(Self::setting_row(
                "settings.scroll_on_input",
                Some("settings.scroll_on_input_desc"),
                Switch::new("toggle-scroll-on-input")
                    .small()
                    .checked(self.settings.terminal_profile.scroll_on_input)
                    .on_click(cx.listener(|this, &checked, _, cx| {
                        this.settings.terminal_profile.scroll_on_input = checked;
                        cx.notify();
                    }))
                    .into_any_element(),
                p,
            ))
            .child(Self::setting_field_row(
                Self::form_field(
                    rust_i18n::t!("settings.scrollback").to_string(),
                    self.modals.settings.scrollback.clone(),
                    p,
                ),
                None,
                p,
            ))
            .into_any_element()
    }

    /// 关键词高亮组：命中高亮开关与关键词列表。
    pub(crate) fn render_settings_highlight(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        v_flex()
            .gap_3()
            .child(Self::setting_row(
                "settings.keyword_highlight",
                Some("settings.keyword_highlight_desc"),
                Switch::new("toggle-keyword-highlight")
                    .checked(self.settings.terminal_profile.highlight.enabled)
                    .on_click(cx.listener(|this, &checked, _, cx| {
                        this.settings.terminal_profile.highlight.enabled = checked;
                        cx.notify();
                    }))
                    .into_any_element(),
                p,
            ))
            .child(Self::setting_field_row(
                Self::form_field(
                    rust_i18n::t!("settings.highlight_keywords").to_string(),
                    self.modals.settings.highlight_keywords.clone(),
                    p,
                ),
                Some(rust_i18n::t!("settings.highlight_keywords_note").to_string()),
                p,
            ))
            .into_any_element()
    }

    /// 自定义按键映射组：每行 `按键 = 转义序列`。
    pub(crate) fn render_settings_key_bindings(
        &mut self,
        p: &Palette,
        _cx: &mut Context<Self>,
    ) -> AnyElement {
        Self::setting_frame(
            v_flex()
                .w_full()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(rust_i18n::t!("settings.key_bindings_desc").to_string()),
                )
                .child(
                    Textarea::new(&self.modals.settings.terminal_key_bindings)
                        .h(px(120.0))
                        .w_full(),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(rust_i18n::t!("settings.key_bindings_hint").to_string()),
                )
                .into_any_element(),
            p,
        )
    }
}
