//! 偏好设置中心之终端预设调色板与按键映射配置页渲染。

use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Icon;
use gpui_kit::component::input::Textarea;
use gpui_kit::*;
use transom_core::{FOLLOW_UI_THEME, preset_themes};

impl AppShell {
    /// 终端预设调色板网格：内置配色卡片与跟随界面主题联动开关。
    pub(crate) fn render_settings_themes(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let themes = preset_themes();
        let current_theme_name = self.settings.terminal_profile.theme_name.clone();

        // "跟随界面主题"卡：终端配色默认随 UI 主题明暗联动（Islands Dark 配
        // Islands 终端色），显式选预设即覆盖。置于列表首位。
        let follow_selected = current_theme_name == FOLLOW_UI_THEME;
        let follow_card = v_flex()
            .id(ElementId::Name("terminal-follow-card".into()))
            .w_full()
            .p_3()
            .rounded_lg()
            .border_1()
            .border_color(if follow_selected {
                p.sidebar_accent_foreground
            } else {
                p.border
            })
            .bg(if follow_selected {
                p.sidebar_accent
            } else {
                p.popover
            })
            .gap_2p5()
            .cursor_pointer()
            .hover(|s| {
                if follow_selected {
                    s
                } else {
                    s.border_color(p.muted_foreground)
                }
            })
            .on_click(cx.listener(|this, _, _, cx| {
                this.settings.terminal_profile.theme_name = FOLLOW_UI_THEME.to_string();
                this.sync_terminal_palette(cx);
                cx.notify();
            }))
            .child(
                h_flex().w_full().justify_between().items_center().child(
                    div()
                        .text_sm()
                        .font_weight(if follow_selected {
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
            .into_any_element();

        let theme_cards: Vec<AnyElement> = themes
            .into_iter()
            .enumerate()
            .map(|(idx, theme)| {
                let is_selected = theme.name == current_theme_name;
                let theme_clone = theme.clone();
                let theme_name = theme.name.clone();

                let dots = h_flex()
                    .gap_1()
                    .children(theme.palette.ansi.iter().take(8).map(|color| {
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
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.settings.terminal_profile.theme_name = theme_clone.name.clone();
                        this.settings.terminal_profile.palette = theme_clone.palette.clone();
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
                    .child(
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
                                            .text_color(Hsla::from(rgb(
                                                theme.palette.ansi[2].to_hex()
                                            )))
                                            .child("user@transom"),
                                    )
                                    .child(
                                        div()
                                            .text_color(Hsla::from(rgb(theme
                                                .palette
                                                .foreground
                                                .to_hex())))
                                            .child(":"),
                                    )
                                    .child(
                                        div()
                                            .text_color(Hsla::from(rgb(
                                                theme.palette.ansi[4].to_hex()
                                            )))
                                            .child("~"),
                                    )
                                    .child(
                                        div()
                                            .text_color(Hsla::from(rgb(theme
                                                .palette
                                                .foreground
                                                .to_hex())))
                                            .child("$ ls -la"),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .gap_3()
                                    .child(
                                        h_flex()
                                            .gap_1()
                                            .child(Icon::new(IconName::Folder).size_3().text_color(
                                                Hsla::from(rgb(theme.palette.ansi[4].to_hex())),
                                            ))
                                            .child(
                                                div()
                                                    .font_weight(FontWeight::BOLD)
                                                    .text_color(Hsla::from(rgb(theme.palette.ansi
                                                        [4]
                                                    .to_hex())))
                                                    .child("Documents"),
                                            ),
                                    )
                                    .child(
                                        h_flex()
                                            .gap_1()
                                            .child(Icon::new(IconName::Folder).size_3().text_color(
                                                Hsla::from(rgb(theme.palette.ansi[2].to_hex())),
                                            ))
                                            .child(
                                                div()
                                                    .font_weight(FontWeight::BOLD)
                                                    .text_color(Hsla::from(rgb(theme.palette.ansi
                                                        [2]
                                                    .to_hex())))
                                                    .child("Downloads"),
                                            ),
                                    )
                                    .child(
                                        h_flex()
                                            .gap_1()
                                            .child(Icon::new(IconName::File).size_3().text_color(
                                                Hsla::from(rgb(theme.palette.foreground.to_hex())),
                                            ))
                                            .child(
                                                div()
                                                    .text_color(Hsla::from(rgb(theme
                                                        .palette
                                                        .foreground
                                                        .to_hex())))
                                                    .child("main.rs"),
                                            ),
                                    ),
                            ),
                    )
                    .into_any_element()
            })
            .collect();

        v_flex()
            .gap_3()
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("settings.terminal_theme_note").to_string()),
            )
            .child(
                v_flex()
                    .gap_2p5()
                    .children([follow_card])
                    .children(theme_cards),
            )
            .into_any_element()
    }

    /// 自定义按键映射组：每行 `按键 = 转义序列`。
    pub(crate) fn render_settings_key_bindings(
        &mut self,
        p: &Palette,
        _cx: &mut Context<Self>,
    ) -> AnyElement {
        v_flex()
            .gap_1()
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("settings.key_bindings_desc").to_string()),
            )
            .child(
                Textarea::new(&self.settings_modal.terminal_key_bindings)
                    .h(px(120.0))
                    .w_full(),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("settings.key_bindings_hint").to_string()),
            )
            .into_any_element()
    }
}
