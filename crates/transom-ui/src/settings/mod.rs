//! 偏好设置中心的动作入口与通用/渲染/快捷键配置页模块。

mod general_page;
mod layout_page;
mod modal;
mod pages;
mod shortcuts_page;

use crate::app::AppShell;
use crate::palette::{Palette, localized_error};
use gpui_kit::base::{Selectable, h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::Button;
use gpui_kit::component::input::InputState;
use gpui_kit::component::switch::Switch;
use gpui_kit::*;
use transom_core::{TerminalCursorShape, TerminalHighlight};
use transom_terminal::TerminalView;

/// 读取字体族输入框：留空表示"不指定"，交给默认字体或系统回退。
fn read_font_family(input: &Entity<InputState>, cx: &App) -> Option<String> {
    let value = input.read(cx).value();
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

impl AppShell {
    pub(crate) fn open_settings_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let profile = self.settings.terminal_profile.clone();
        self.settings_font_family.update(cx, |input, cx| {
            input.set_value(profile.font_family.unwrap_or_default(), window, cx);
        });
        self.settings_cjk_font.update(cx, |input, cx| {
            input.set_value(profile.cjk_font_family.unwrap_or_default(), window, cx);
        });
        let font_size = self.settings.terminal_profile.font_size.to_string();
        self.settings_font_size.update(cx, |input, cx| {
            input.set_value(&font_size, window, cx);
        });
        let line_height = self.settings.terminal_profile.line_height.to_string();
        self.settings_line_height.update(cx, |input, cx| {
            input.set_value(&line_height, window, cx);
        });
        self.settings_word_separators.update(cx, |input, cx| {
            input.set_value(
                self.settings.terminal_profile.word_separators.clone(),
                window,
                cx,
            );
        });
        self.settings_highlight_keywords.update(cx, |input, cx| {
            input.set_value(profile.highlight.keywords_joined(), window, cx);
        });
        let notify_threshold = self.settings.window.notify_threshold_secs.to_string();
        self.settings_notify_threshold.update(cx, |input, cx| {
            input.set_value(notify_threshold, window, cx);
        });
        let scrollback = self.settings.terminal_profile.scrollback.to_string();
        self.settings_scrollback.update(cx, |input, cx| {
            input.set_value(&scrollback, window, cx);
        });
        self.show_settings_modal = true;
        cx.notify();
    }

    pub(crate) fn save_settings(&mut self, cx: &mut Context<Self>) {
        self.settings.terminal_profile.font_family =
            read_font_family(&self.settings_font_family, cx);
        self.settings.terminal_profile.cjk_font_family =
            read_font_family(&self.settings_cjk_font, cx);
        // 关键词列表先解析再清洗（去空、不分大小写去重、限长限条数），
        // 让"界面上看到的"与"存进文件的"是同一份。
        self.settings.terminal_profile.highlight.keywords = TerminalHighlight {
            enabled: self.settings.terminal_profile.highlight.enabled,
            keywords: TerminalHighlight::parse_keywords(
                self.settings_highlight_keywords.read(cx).value().as_ref(),
            ),
        }
        .sanitized()
        .keywords;
        if let Ok(font_size) = self
            .settings_font_size
            .read(cx)
            .value()
            .trim()
            .parse::<f32>()
            && (8.0..=36.0).contains(&font_size)
        {
            self.settings.terminal_profile.font_size = font_size;
        }
        if let Ok(line_height) = self
            .settings_line_height
            .read(cx)
            .value()
            .trim()
            .parse::<f32>()
            && (1.0..=2.5).contains(&line_height)
        {
            self.settings.terminal_profile.line_height = line_height;
        }
        // 空串是有值意义的（不按任何字符断词），因此这里不 trim、也不拒绝空值。
        self.settings.terminal_profile.word_separators =
            self.settings_word_separators.read(cx).value().to_string();
        if let Some(threshold) = crate::notify::parse_notify_threshold(
            self.settings_notify_threshold.read(cx).value().as_ref(),
        ) {
            self.settings.window.notify_threshold_secs = threshold;
        }
        if let Ok(scrollback) = self
            .settings_scrollback
            .read(cx)
            .value()
            .trim()
            .parse::<usize>()
            && (100..=100_000).contains(&scrollback)
        {
            self.settings.terminal_profile.scrollback = scrollback;
        }
        if let Err(error) = self.services.settings_store.save(&self.settings) {
            self.error = Some(localized_error(&error));
        } else {
            self.apply_appearance_to_live_panes(cx);
            self.apply_shortcut_settings(cx);
            self.status = Some(rust_i18n::t!("status.ready").to_string());
        }
        self.show_settings_modal = false;
        cx.notify();
    }

    /// 把外观偏好下发到所有已打开的窗格。
    ///
    /// 不做这件事的话，改完字体要重开标签才见效，等于"设置了但没反应"。
    /// 会话本身也带 profile，但当前没有任何界面在会话级别改字体，因此以全局偏好为准；
    /// 将来开放会话级字体时，这里必须改成只补空白、不覆盖会话选择。
    fn apply_appearance_to_live_panes(&self, cx: &mut Context<Self>) {
        let profile = self.settings.terminal_profile.clone();
        // 先收集实体再逐个更新：遍历 self.tabs 与逐条 update 会同时借用 self。
        let terminals: Vec<Entity<TerminalView>> = self
            .tabs
            .iter()
            .filter_map(|tab| tab.terminal.clone())
            .collect();
        for terminal in terminals {
            terminal.update(cx, |view, cx| view.apply_appearance(&profile, cx));
        }
    }

    pub(crate) fn render_settings_rendering(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        v_flex()
            .gap_4()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::BOLD)
                    .child(rust_i18n::t!("settings.appearance").to_string()),
            )
            .child(Self::form_field(
                rust_i18n::t!("settings.font_family").to_string(),
                self.settings_font_family.clone(),
                p,
            ))
            .child(Self::form_field(
                rust_i18n::t!("settings.cjk_font").to_string(),
                self.settings_cjk_font.clone(),
                p,
            ))
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("settings.cjk_font_note").to_string()),
            )
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .py_1()
                    .child(
                        div()
                            .text_sm()
                            .child(rust_i18n::t!("settings.keyword_highlight").to_string()),
                    )
                    .child(
                        Switch::new("toggle-keyword-highlight")
                            .checked(self.settings.terminal_profile.highlight.enabled)
                            .on_click(cx.listener(|this, &checked, _, cx| {
                                this.settings.terminal_profile.highlight.enabled = checked;
                                cx.notify();
                            })),
                    ),
            )
            .child(Self::form_field(
                rust_i18n::t!("settings.highlight_keywords").to_string(),
                self.settings_highlight_keywords.clone(),
                p,
            ))
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("settings.highlight_keywords_note").to_string()),
            )
            .child(Self::form_field(
                rust_i18n::t!("settings.font_size").to_string(),
                self.settings_font_size.clone(),
                p,
            ))
            .child(Self::form_field(
                rust_i18n::t!("settings.line_height").to_string(),
                self.settings_line_height.clone(),
                p,
            ))
            .child(Self::form_field(
                rust_i18n::t!("settings.scrollback").to_string(),
                self.settings_scrollback.clone(),
                p,
            ))
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .py_1()
                    .child(
                        div()
                            .text_sm()
                            .child(rust_i18n::t!("settings.cursor_shape").to_string()),
                    )
                    .child(
                        h_flex().gap_1().children(
                            [
                                (
                                    "cursor-block",
                                    "settings.cursor_block",
                                    TerminalCursorShape::Block,
                                ),
                                (
                                    "cursor-bar",
                                    "settings.cursor_bar",
                                    TerminalCursorShape::Bar,
                                ),
                                (
                                    "cursor-underline",
                                    "settings.cursor_underline",
                                    TerminalCursorShape::Underline,
                                ),
                            ]
                            .into_iter()
                            .map(|(id, key, shape)| {
                                Button::new(id)
                                    .small()
                                    .label(rust_i18n::t!(key).to_string())
                                    .selected(self.settings.terminal_profile.cursor_shape == shape)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.settings.terminal_profile.cursor_shape = shape;
                                        cx.notify();
                                    }))
                            }),
                        ),
                    ),
            )
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .py_1()
                    .child(
                        div()
                            .text_sm()
                            .child(rust_i18n::t!("settings.cursor_blink").to_string()),
                    )
                    .child(
                        Switch::new("toggle-cursor-blink")
                            .small()
                            .checked(self.settings.terminal_profile.cursor_blink)
                            .on_click(cx.listener(|this, &checked, _, cx| {
                                this.settings.terminal_profile.cursor_blink = checked;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .py_1()
                    .child(
                        div()
                            .text_sm()
                            .child(rust_i18n::t!("settings.bold_is_bright").to_string()),
                    )
                    .child(
                        Switch::new("toggle-bold-is-bright")
                            .small()
                            .checked(self.settings.terminal_profile.bold_is_bright)
                            .on_click(cx.listener(|this, &checked, _, cx| {
                                this.settings.terminal_profile.bold_is_bright = checked;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .py_1()
                    .child(
                        div()
                            .text_sm()
                            .child(rust_i18n::t!("settings.scroll_on_input").to_string()),
                    )
                    .child(
                        Switch::new("toggle-scroll-on-input")
                            .small()
                            .checked(self.settings.terminal_profile.scroll_on_input)
                            .on_click(cx.listener(|this, &checked, _, cx| {
                                this.settings.terminal_profile.scroll_on_input = checked;
                                cx.notify();
                            })),
                    ),
            )
            .into_any_element()
    }
}
