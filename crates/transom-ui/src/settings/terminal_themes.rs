//! 偏好设置中心之终端配色页：预设调色板网格与"跟随界面主题"联动。
//!
//! 从 `terminal_page.rs` 拆出：配色卡片连同主题预览示例占了该页近一半篇幅，
//! 与字体、光标等设置行没有共享状态，单独成文件更便于对照 `preset_themes`。

use crate::app::app_settings;
use crate::app::update_app_settings;
use crate::palette::Palette;
use crate::settings::SettingsEvent;
use crate::settings::SettingsView;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Icon;
use gpui_kit::*;
use transom_core::{FOLLOW_UI_THEME, preset_themes};

impl SettingsView {
    /// 终端预设调色板网格：内置配色卡片与跟随界面主题联动开关。
    pub(crate) fn render_settings_themes(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let themes = preset_themes();
        let current_theme_name = app_settings(cx).terminal_profile.theme_name.clone();
        let follow_selected = current_theme_name == FOLLOW_UI_THEME;

        let follow_card = self.render_follow_theme_card(follow_selected, p, cx);

        let dark_cards: Vec<AnyElement> = themes
            .iter()
            .filter(|theme| theme.mode == transom_core::PaletteMode::Dark)
            .enumerate()
            .map(|(idx, theme)| {
                self.render_theme_card(idx, theme, theme.name == current_theme_name, p, cx)
            })
            .collect();
        let light_cards: Vec<AnyElement> = themes
            .iter()
            .filter(|theme| theme.mode == transom_core::PaletteMode::Light)
            .enumerate()
            .map(|(idx, theme)| {
                self.render_theme_card(
                    dark_cards.len() + idx,
                    theme,
                    theme.name == current_theme_name,
                    p,
                    cx,
                )
            })
            .collect();

        // 主题卡是自带边框的卡片网格，不按行画线；整块作为一项，在组末补一条。
        crate::widgets::setting_frame(
            v_flex()
                .w_full()
                .gap_3()
                .child(
                    div()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(rust_i18n::t!("settings.terminal_theme_note").to_string()),
                )
                .child(
                    v_flex()
                        .gap_3()
                        .children([follow_card])
                        .child(preset_theme_section(
                            rust_i18n::t!("settings.theme_group_dark").to_string(),
                            dark_cards,
                            p,
                        ))
                        .child(preset_theme_section(
                            rust_i18n::t!("settings.theme_group_light").to_string(),
                            light_cards,
                            p,
                        )),
                )
                .child(self.render_terminal_contrast_row(p, cx))
                .into_any_element(),
            p,
        )
    }

    /// 终端正文最小显示对比度。
    ///
    /// 修正发生在渲染层，不改变 ANSI、复制文本和会话记录；显式终端配色与
    /// 跟随界面主题都共享这一设置。
    fn render_terminal_contrast_row(&mut self, p: &Palette, cx: &mut Context<Self>) -> AnyElement {
        let ratio = app_settings(cx).terminal_profile.minimum_contrast_ratio;
        let current = if (ratio - 3.0).abs() < 0.01 {
            "3"
        } else if (ratio - 4.5).abs() < 0.01 {
            "4.5"
        } else if (ratio - 7.0).abs() < 0.01 {
            "7"
        } else if (ratio - 21.0).abs() < 0.01 {
            "21"
        } else {
            "1"
        };
        let options = ["1", "3", "4.5", "7", "21"]
            .into_iter()
            .map(|value| {
                let key = match value {
                    "3" => "settings.terminal_contrast_minimum",
                    "4.5" => "settings.terminal_contrast_aa",
                    "7" => "settings.terminal_contrast_aaa",
                    "21" => "settings.terminal_contrast_maximum",
                    _ => "settings.terminal_contrast_off",
                };
                (value, rust_i18n::t!(key).to_string())
            })
            .collect();
        let dropdown = Self::setting_dropdown(
            "dropdown-terminal-minimum-contrast",
            current,
            options,
            |_this, value, cx| {
                let ratio = value.parse::<f32>().unwrap_or(1.0).clamp(1.0, 21.0);
                update_app_settings(cx, |settings| {
                    settings.terminal_profile.minimum_contrast_ratio = ratio;
                });
                cx.emit(SettingsEvent::ProfilePreview);
                cx.notify();
            },
            cx,
        );
        crate::widgets::setting_row(
            "settings.terminal_minimum_contrast",
            Some("settings.terminal_minimum_contrast_desc"),
            dropdown,
            p,
        )
    }

    /// "跟随界面主题"卡：选中态与说明固定，点击写回哨兵值。
    fn render_follow_theme_card(
        &self,
        selected: bool,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // "跟随界面主题"卡：终端配色默认随 UI 主题明暗联动（Islands Dark 配
        // Islands 终端色），显式选预设即覆盖。置于列表首位。
        v_flex()
            .id(ElementId::Name("terminal-follow-card".into()))
            .w_full()
            .p_3()
            .rounded_lg()
            .border_1()
            .border_color(if selected {
                p.sidebar_accent_foreground
            } else {
                p.border
            })
            .bg(if selected {
                p.sidebar_accent
            } else {
                p.popover
            })
            .gap_2p5()
            .cursor_pointer()
            .hover(|s| {
                if selected {
                    s
                } else {
                    s.border_color(p.muted_foreground)
                }
            })
            .on_click(cx.listener(|this, _, _, cx| {
                update_app_settings(cx, |s| {
                    s.terminal_profile.theme_name = FOLLOW_UI_THEME.to_string()
                });
                this.sync_terminal_palette(cx);
                cx.notify();
            }))
            .child(
                h_flex().w_full().justify_between().items_center().child(
                    div()
                        .text_sm()
                        .font_weight(if selected {
                            FontWeight::BOLD
                        } else {
                            FontWeight::NORMAL
                        })
                        .child(rust_i18n::t!("settings.terminal_follow_ui").to_string()),
                ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("settings.terminal_follow_ui_note").to_string()),
            )
            .into_any_element()
    }

    /// 单个预设法卡片：色点、名称、选中圈与终端预览。
    fn render_theme_card(
        &self,
        idx: usize,
        theme: &transom_core::PresetTheme,
        is_selected: bool,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme_clone = theme.clone();
        let theme_name = theme.name.clone();

        let dots = h_flex()
            .gap_1()
            .children(theme.palette.ansi.iter().map(|color| {
                div()
                    .size(px(10.0))
                    .rounded_full()
                    .bg(Hsla::from(rgb(color.to_hex())))
            }));

        v_flex()
            .id(ElementId::NamedInteger("theme-card".into(), idx as u64))
            .w_full()
            .p_3()
            .rounded(p.radius.lg)
            .border_1()
            .border_color(if is_selected {
                p.sidebar_accent_foreground
            } else {
                p.border
            })
            .bg(if is_selected {
                p.sidebar_accent
            } else {
                p.popover
            })
            .gap_2p5()
            .cursor_pointer()
            .hover(|s| {
                if is_selected {
                    s
                } else {
                    s.border_color(p.muted_foreground)
                }
            })
            .on_click(cx.listener(move |_this, _, _, cx| {
                update_app_settings(cx, |s| {
                    s.terminal_profile.theme_name = theme_clone.name.clone()
                });
                update_app_settings(cx, |s| {
                    s.terminal_profile.palette = theme_clone.palette.clone()
                });
                cx.notify();
            }))
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .items_center()
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .size(px(12.0))
                                    .rounded_full()
                                    .border_2()
                                    .border_color(if is_selected {
                                        p.sidebar_accent_foreground
                                    } else {
                                        p.border
                                    })
                                    .bg(if is_selected {
                                        p.sidebar_accent_foreground
                                    } else {
                                        transparent_black()
                                    }),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::BOLD)
                                    .child(theme_name),
                            ),
                    )
                    .child(dots),
            )
            .child(terminal_preview(theme, p))
            .into_any_element()
    }
}

fn preset_theme_section(title: String, cards: Vec<AnyElement>, p: &Palette) -> AnyElement {
    v_flex()
        .w_full()
        .gap_2p5()
        .child(div().text_xs().text_color(p.muted_foreground).child(title))
        .children(cards)
        .into_any_element()
}

/// 终端配色预览：背景、提示符、目录与文件行。
fn terminal_preview(theme: &transom_core::PresetTheme, p: &Palette) -> AnyElement {
    v_flex()
        .w_full()
        .p_2p5()
        .rounded(p.radius.md)
        .bg(Hsla::from(rgb(theme.palette.background.to_hex())))
        .text_xs()
        .gap_1()
        .child(
            h_flex()
                .gap_1()
                .child(
                    div()
                        .text_color(Hsla::from(rgb(theme.palette.ansi[2].to_hex())))
                        .child("user@transom"),
                )
                .child(
                    div()
                        .text_color(Hsla::from(rgb(theme.palette.foreground.to_hex())))
                        .child(":"),
                )
                .child(
                    div()
                        .text_color(Hsla::from(rgb(theme.palette.ansi[4].to_hex())))
                        .child("~"),
                )
                .child(
                    div()
                        .text_color(Hsla::from(rgb(theme.palette.foreground.to_hex())))
                        .child("$ ls -la"),
                ),
        )
        .child(
            h_flex()
                .gap_3()
                .child(
                    h_flex()
                        .gap_1()
                        .child(
                            Icon::new(IconName::Folder)
                                .size_3()
                                .text_color(Hsla::from(rgb(theme.palette.ansi[4].to_hex()))),
                        )
                        .child(
                            div()
                                .font_weight(FontWeight::BOLD)
                                .text_color(Hsla::from(rgb(theme.palette.ansi[4].to_hex())))
                                .child("Documents"),
                        ),
                )
                .child(
                    h_flex()
                        .gap_1()
                        .child(
                            Icon::new(IconName::Folder)
                                .size_3()
                                .text_color(Hsla::from(rgb(theme.palette.ansi[2].to_hex()))),
                        )
                        .child(
                            div()
                                .font_weight(FontWeight::BOLD)
                                .text_color(Hsla::from(rgb(theme.palette.ansi[2].to_hex())))
                                .child("Downloads"),
                        ),
                )
                .child(
                    h_flex()
                        .gap_1()
                        .child(
                            Icon::new(IconName::File)
                                .size_3()
                                .text_color(Hsla::from(rgb(theme.palette.foreground.to_hex()))),
                        )
                        .child(
                            div()
                                .text_color(Hsla::from(rgb(theme.palette.foreground.to_hex())))
                                .child("main.rs"),
                        ),
                ),
        )
        .into_any_element()
}
