//! 偏好设置中心之主题配色页、鼠标剪贴板页与标签页窗口页渲染。

use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{Selectable, h_flex, v_flex};
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::button::Button;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::theme::{ThemeConfig, ThemeMode, ThemeRegistry, try_parse_color};
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use std::rc::Rc;
use transom_core::{PaletteMode, UiDensity, UiThemeMode, preset_themes};

impl AppShell {
    pub(crate) fn render_settings_themes(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let themes = preset_themes();
        let current_theme_name = self.settings.terminal_profile.theme_name.clone();

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

        // 组标题由 SettingGroup 提供（"终端配色"），页内不再重复标题行。
        v_flex()
            .gap_3()
            .child(v_flex().gap_2p5().children(theme_cards))
            .into_any_element()
    }

    /// 界面主题组：明暗模式、密度档位与全局主题卡片。
    ///
    /// 选择即预览：每次改动经 [`AppShell::update_appearance`] 立即应用到全局
    /// 主题，保存/取消分别负责落盘与回滚。选择主题卡会把明暗模式一并切到该
    /// 主题自带的属性——主题本身携带明暗语义，只记名字不改模式会出现"选了
    /// 深色主题但界面还是浅色"的假死状态。
    pub(crate) fn render_settings_ui_theme(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let themes: Vec<Rc<ThemeConfig>> = ThemeRegistry::global(cx)
            .sorted_themes()
            .into_iter()
            .cloned()
            .collect();
        let active_name = cx.theme().theme_name().clone();

        let mode_row = h_flex()
            .justify_between()
            .items_center()
            .py_1()
            .child(
                div()
                    .text_sm()
                    .child(rust_i18n::t!("settings.ui_mode").to_string()),
            )
            .child(
                h_flex().gap_1().children(
                    [
                        (
                            "ui-mode-system",
                            "settings.ui_mode_system",
                            UiThemeMode::System,
                        ),
                        (
                            "ui-mode-light",
                            "settings.ui_mode_light",
                            UiThemeMode::Light,
                        ),
                        ("ui-mode-dark", "settings.ui_mode_dark", UiThemeMode::Dark),
                    ]
                    .into_iter()
                    .map(|(id, key, mode)| {
                        Button::new(id)
                            .small()
                            .label(rust_i18n::t!(key).to_string())
                            .selected(self.settings.appearance.mode == mode)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.update_appearance(|appearance| appearance.mode = mode, cx);
                            }))
                    }),
                ),
            );

        let density_row = h_flex()
            .justify_between()
            .items_center()
            .py_1()
            .child(
                div()
                    .text_sm()
                    .child(rust_i18n::t!("settings.density").to_string()),
            )
            .child(
                h_flex()
                    .gap_1()
                    .children(UiDensity::ALL.into_iter().map(|density| {
                        Button::new(density.label_key())
                            .small()
                            .label(rust_i18n::t!(density.label_key()).to_string())
                            .selected(self.settings.appearance.density == density)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.update_appearance(
                                    |appearance| appearance.density = density,
                                    cx,
                                );
                            }))
                    })),
            );

        let cards: Vec<AnyElement> = themes
            .iter()
            .enumerate()
            .map(|(idx, config)| {
                let is_active = config.name == active_name;
                let name = config.name.to_string();
                // 点击闭包要拿走一份名称，卡片标题继续用原值，各持一份。
                let click_name = name.clone();
                let mode = config.mode;
                let mode_label = match mode {
                    ThemeMode::Dark => rust_i18n::t!("settings.theme_mode_dark").to_string(),
                    ThemeMode::Light => rust_i18n::t!("settings.theme_mode_light").to_string(),
                };
                h_flex()
                    .id(ElementId::NamedInteger("ui-theme-card".into(), idx as u64))
                    .w_full()
                    .px_3()
                    .py_2()
                    .gap_3()
                    .items_center()
                    .rounded(p.radius.md)
                    .border_1()
                    .border_color(if is_active { p.primary } else { p.border })
                    .bg(if is_active { p.muted } else { p.popover })
                    .cursor_pointer()
                    .hover(|s| {
                        if is_active {
                            s
                        } else {
                            s.border_color(p.muted_foreground)
                        }
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let theme_mode = match mode {
                            ThemeMode::Dark => UiThemeMode::Dark,
                            ThemeMode::Light => UiThemeMode::Light,
                        };
                        this.update_appearance(
                            |appearance| {
                                appearance.ui_theme = click_name.clone();
                                appearance.mode = theme_mode;
                            },
                            cx,
                        );
                    }))
                    .child(theme_color_dots(config))
                    .child(
                        div()
                            .text_sm()
                            .font_weight(if is_active {
                                FontWeight::BOLD
                            } else {
                                FontWeight::NORMAL
                            })
                            .child(name),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child(mode_label),
                    )
                    .into_any_element()
            })
            .collect();

        // 明暗反差提示：界面主题与终端配色一个深一个浅时给出文案，不改用户选择。
        let terminal_mode = preset_themes()
            .into_iter()
            .find(|theme| theme.name == self.settings.terminal_profile.theme_name)
            .map(|theme| theme.mode);
        let contrast_hint = matches!(
            (cx.theme().is_dark(), terminal_mode),
            (true, Some(PaletteMode::Light)) | (false, Some(PaletteMode::Dark))
        );

        v_flex()
            .gap_3()
            .child(mode_row)
            .child(density_row)
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("settings.ui_theme_note").to_string()),
            )
            .child(v_flex().gap_2().children(cards))
            .when(contrast_hint, |this| {
                this.child(
                    h_flex()
                        .gap_2()
                        .items_start()
                        .child(
                            Icon::new(IconName::TriangleAlert)
                                .size_3()
                                .text_color(p.warning),
                        )
                        .child(
                            div()
                                .flex_1()
                                .text_xs()
                                .text_color(p.warning)
                                .child(rust_i18n::t!("settings.appearance_contrast").to_string()),
                        ),
                )
            })
            .into_any_element()
    }
}

/// 从主题配置解析三个预览色点（背景、前景、强调色）；解析失败显示透明点。
///
/// 主题文件的颜色值是字符串（hex 或 Tailwind 名），交给 gpui-kit 的解析器
/// 统一处理，解析规则与实际应用主题时一致，避免预览与真实效果色差。
fn theme_color_dots(config: &ThemeConfig) -> AnyElement {
    let entries = [
        config.colors.background.as_deref(),
        config.colors.foreground.as_deref(),
        config.colors.primary.as_deref(),
    ];
    h_flex()
        .gap_1()
        .children(entries.into_iter().map(|value| {
            let color = value.and_then(|value| try_parse_color(value).ok());
            div()
                .size(px(10.0))
                .rounded_full()
                .bg(color.unwrap_or(transparent_black()))
        }))
        .into_any_element()
}

impl AppShell {
    pub(crate) fn render_settings_mouse_clipboard(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let is_paste_mode = self.settings.terminal_interaction.right_click_paste;

        v_flex()
            .gap_4()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::BOLD)
                    .child(rust_i18n::t!("settings.interaction").to_string()),
            )
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .py_1()
                    .child(
                        div()
                            .text_sm()
                            .child(rust_i18n::t!("settings.right_click_action").to_string()),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .child(
                                Button::new("rc-menu")
                                    .small()
                                    .label(rust_i18n::t!("settings.right_click_menu").to_string())
                                    .selected(!is_paste_mode)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.settings.terminal_interaction.right_click_paste =
                                            false;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("rc-paste")
                                    .small()
                                    .label(
                                        rust_i18n::t!("settings.right_click_paste_action")
                                            .to_string(),
                                    )
                                    .selected(is_paste_mode)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.settings.terminal_interaction.right_click_paste = true;
                                        cx.notify();
                                    })),
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
                            .child(rust_i18n::t!("settings.middle_click_paste").to_string()),
                    )
                    .child(
                        Switch::new("toggle-middle-click-paste")
                            .small()
                            .checked(self.settings.terminal_interaction.middle_click_paste)
                            .on_click(cx.listener(|this, &checked, _, cx| {
                                this.settings.terminal_interaction.middle_click_paste = checked;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::BOLD)
                    .pt_2()
                    .child(rust_i18n::t!("settings.selection").to_string()),
            )
            .child(Self::form_field(
                rust_i18n::t!("settings.word_separators").to_string(),
                self.settings_modal.word_separators.clone(),
                p,
            ))
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("settings.word_separators_note").to_string()),
            )
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::BOLD)
                    .pt_2()
                    .child(rust_i18n::t!("settings.clipboard").to_string()),
            )
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .py_1()
                    .child(
                        div()
                            .text_sm()
                            .child(rust_i18n::t!("settings.copy_on_select").to_string()),
                    )
                    .child(
                        Switch::new("toggle-copy-on-select")
                            .small()
                            .checked(self.settings.terminal_interaction.copy_on_select)
                            .on_click(cx.listener(|this, &checked, _, cx| {
                                this.settings.terminal_interaction.copy_on_select = checked;
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
                            .child(rust_i18n::t!("settings.multiline_paste").to_string()),
                    )
                    .child(
                        Switch::new("toggle-multiline-paste")
                            .small()
                            .checked(self.settings.terminal_interaction.multiline_paste_confirm)
                            .on_click(cx.listener(|this, &checked, _, cx| {
                                this.settings.terminal_interaction.multiline_paste_confirm =
                                    checked;
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
                            .child(rust_i18n::t!("settings.trim_trailing_whitespace").to_string()),
                    )
                    .child(
                        Switch::new("toggle-trim-whitespace")
                            .small()
                            .checked(self.settings.terminal_interaction.trim_trailing_whitespace)
                            .on_click(cx.listener(|this, &checked, _, cx| {
                                this.settings.terminal_interaction.trim_trailing_whitespace =
                                    checked;
                                cx.notify();
                            })),
                    ),
            )
            .into_any_element()
    }

    pub(crate) fn render_settings_tabs_window(
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
                    .child(rust_i18n::t!("settings.tab_tabs_window").to_string()),
            )
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .py_1()
                    .child(
                        div()
                            .text_sm()
                            .child(rust_i18n::t!("settings.show_tab_close_button").to_string()),
                    )
                    .child(
                        Switch::new("toggle-tab-close-btn")
                            .small()
                            .checked(self.settings.window.show_tab_close_button)
                            .on_click(cx.listener(|this, &checked, _, cx| {
                                this.settings.window.show_tab_close_button = checked;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .py_1()
                    .child(div().text_sm().child(
                        rust_i18n::t!("settings.close_window_on_all_tabs_closed").to_string(),
                    ))
                    .child(
                        Switch::new("toggle-close-on-last-tab")
                            .small()
                            .checked(self.settings.window.close_window_on_all_tabs_closed)
                            .on_click(cx.listener(|this, &checked, _, cx| {
                                this.settings.window.close_window_on_all_tabs_closed = checked;
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
                        v_flex()
                            .gap_1()
                            .child(div().text_sm().child("记忆窗口大小与屏幕位置"))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(p.muted_foreground)
                                    .child("重新打开 Transom 时恢复上次关闭时的窗口尺寸和屏幕坐标"),
                            ),
                    )
                    .child(
                        Switch::new("toggle-remember-bounds")
                            .small()
                            .checked(self.settings.window.remember_window_bounds)
                            .on_click(cx.listener(|this, &checked, _, cx| {
                                this.settings.window.remember_window_bounds = checked;
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
                        v_flex()
                            .gap_1()
                            .child(div().text_sm().child("启动时恢复上次会话标签页"))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(p.muted_foreground)
                                    .child("启动时自动重新连接并打开上次退出时未关闭的终端会话"),
                            ),
                    )
                    .child(
                        Switch::new("toggle-restore-tabs")
                            .small()
                            .checked(self.settings.window.restore_previous_tabs)
                            .on_click(cx.listener(|this, &checked, _, cx| {
                                this.settings.window.restore_previous_tabs = checked;
                                cx.notify();
                            })),
                    ),
            )
            .into_any_element()
    }
}
