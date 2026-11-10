//! 偏好设置中心之终端字体、光标与按键映射配置页渲染。

use super::FontFieldSpec;
use crate::app::app_settings;
use crate::app::update_app_settings;
use crate::divider::{DividerEdge, DividerStyled};
use crate::palette::Palette;
use crate::settings::SettingsView;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::input::Textarea;
use gpui_kit::component::switch::Switch;
use gpui_kit::*;
use transom_core::TerminalCursorShape;

impl SettingsView {
    /// 终端字体组：终端字体族、CJK 字体、字号与行高。
    pub(crate) fn render_settings_terminal_font(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let stepper_size = Self::setting_stepper(&self.state.font_size, px(110.0));
        let stepper_height = Self::setting_stepper(&self.state.line_height, px(110.0));
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
                    select: self.state.font_family.clone(),
                    note_key: None,
                    warn_if_chinese_unsupported: false,
                },
                p,
                cx,
            ))
            .child(Self::setting_font_field(
                FontFieldSpec {
                    id: "terminal-cjk-font",
                    label: rust_i18n::t!("settings.cjk_font").to_string(),
                    select: self.state.cjk_font.clone(),
                    note_key: Some("settings.cjk_font_note"),
                    warn_if_chinese_unsupported: true,
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
        v_flex()
            .gap_2()
            .child(self.render_cursor_shape_row(p, cx))
            .child(switch_setting_row(
                SwitchSettingRow {
                    key: "settings.cursor_blink",
                    desc: "settings.cursor_blink_desc",
                    id: "toggle-cursor-blink",
                    checked: app_settings(cx).terminal_profile.cursor_blink,
                    apply: |s, value| s.terminal_profile.cursor_blink = value,
                },
                p,
                cx,
            ))
            .child(switch_setting_row(
                SwitchSettingRow {
                    key: "settings.bold_is_bright",
                    desc: "settings.bold_is_bright_desc",
                    id: "toggle-bold-is-bright",
                    checked: app_settings(cx).terminal_profile.bold_is_bright,
                    apply: |s, value| s.terminal_profile.bold_is_bright = value,
                },
                p,
                cx,
            ))
            .child(switch_setting_row(
                SwitchSettingRow {
                    key: "settings.alternate_scroll",
                    desc: "settings.alternate_scroll_desc",
                    id: "toggle-alternate-scroll",
                    checked: app_settings(cx).terminal_profile.alternate_scroll,
                    apply: |s, value| s.terminal_profile.alternate_scroll = value,
                },
                p,
                cx,
            ))
            .child(switch_setting_row(
                SwitchSettingRow {
                    key: "settings.scroll_on_input",
                    desc: "settings.scroll_on_input_desc",
                    id: "toggle-scroll-on-input",
                    checked: app_settings(cx).terminal_profile.scroll_on_input,
                    apply: |s, value| s.terminal_profile.scroll_on_input = value,
                },
                p,
                cx,
            ))
            .child(crate::widgets::setting_field_row(
                crate::widgets::form_field(
                    rust_i18n::t!("settings.scrollback").to_string(),
                    self.state.scrollback.clone(),
                    p,
                ),
                None,
                p,
            ))
            .into_any_element()
    }

    /// 光标形状下拉行：块/竖线/下划线三档。
    fn render_cursor_shape_row(&mut self, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let cursor_shape = app_settings(cx).terminal_profile.cursor_shape;
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
            |_this, shape, cx| {
                update_app_settings(cx, |s| s.terminal_profile.cursor_shape = shape);
                cx.notify();
            },
            cx,
        );

        crate::widgets::setting_row(
            "settings.cursor_shape",
            Some("settings.cursor_shape_desc"),
            cursor_dropdown,
            p,
        )
    }

    /// 关键词高亮组：命中高亮开关与关键词列表。
    pub(crate) fn render_settings_highlight(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        v_flex()
            .gap_3()
            .child(crate::widgets::setting_row(
                "settings.keyword_highlight",
                Some("settings.keyword_highlight_desc"),
                Switch::new("toggle-keyword-highlight")
                    .checked(app_settings(cx).terminal_profile.highlight.enabled)
                    .on_click(cx.listener(|_this, &checked, _, cx| {
                        update_app_settings(cx, |s| s.terminal_profile.highlight.enabled = checked);
                        cx.notify();
                    }))
                    .into_any_element(),
                p,
            ))
            .child(crate::widgets::setting_field_row(
                crate::widgets::form_field(
                    rust_i18n::t!("settings.highlight_keywords").to_string(),
                    self.state.highlight_keywords.clone(),
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
        crate::widgets::setting_frame(
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
                    Textarea::new(&self.state.terminal_key_bindings)
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

/// 单个开关设置行：标题/说明 + 开关，点击写回终端配置。
struct SwitchSettingRow {
    key: &'static str,
    desc: &'static str,
    id: &'static str,
    checked: bool,
    apply: fn(&mut transom_core::AppSettings, bool),
}

fn switch_setting_row(
    row: SwitchSettingRow,
    p: &Palette,
    cx: &mut Context<SettingsView>,
) -> AnyElement {
    let apply = row.apply;
    crate::widgets::setting_row(
        row.key,
        Some(row.desc),
        Switch::new(row.id)
            .small()
            .checked(row.checked)
            .on_click(cx.listener(move |_this, &checked, _, cx| {
                update_app_settings(cx, |s| apply(s, checked));
                cx.notify();
            }))
            .into_any_element(),
        p,
    )
}
