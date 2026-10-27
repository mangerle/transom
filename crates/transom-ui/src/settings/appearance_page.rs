//! 偏好设置中心之界面主题、明暗模式与密度档位配置页渲染。

use super::slider_percent;
use crate::app::AppShell;
use crate::palette::Palette;
use crate::window_appearance::DEFAULT_WINDOW_TRANSPARENCY;
use gpui_kit::assets::IconName;
use gpui_kit::base::{Selectable, h_flex, v_flex};
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::button::Button;
use gpui_kit::component::slider::{Slider, SliderState};
use gpui_kit::component::theme::{ThemeConfig, ThemeMode, ThemeRegistry, try_parse_color};
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use std::rc::Rc;
use transom_core::{PaletteMode, UiDensity, UiThemeMode, WindowBackgroundMode, preset_themes};

impl AppShell {
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

    /// 界面字体组：界面字体族覆盖；字号由配色方案页的密度档驱动。
    pub(crate) fn render_settings_ui_font(
        &mut self,
        p: &Palette,
        _cx: &mut Context<Self>,
    ) -> AnyElement {
        v_flex()
            .gap_2()
            .child(Self::form_field(
                rust_i18n::t!("settings.ui_font_family").to_string(),
                self.settings_modal.ui_font_family.clone(),
                p,
            ))
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("settings.ui_font_note").to_string()),
            )
            .into_any_element()
    }

    /// 窗口外观组：背景模式与整体透明度。
    pub(crate) fn render_settings_window_appearance(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mode = self.settings.window.window_background;
        let transparency =
            slider_percent(&self.settings_modal.window_transparency.read(cx).value());
        let mode_buttons = h_flex()
            .gap_1()
            .children(
                [
                    (
                        "window-bg-transparent",
                        "settings.window_bg_transparent",
                        WindowBackgroundMode::Transparent,
                    ),
                    (
                        "window-bg-blurred",
                        "settings.window_bg_blurred",
                        WindowBackgroundMode::Blurred,
                    ),
                ]
                .into_iter()
                .map(|(id, key, choice)| {
                    Button::new(id)
                        .small()
                        .label(rust_i18n::t!(key).to_string())
                        .selected(mode == choice)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.set_window_background_mode(choice, window, cx);
                        }))
                        .into_any_element()
                }),
            )
            .into_any_element();
        v_flex()
            .gap_2()
            .child(Self::setting_row(
                "settings.window_background",
                Some("settings.window_background_desc"),
                mode_buttons,
                p,
            ))
            .child(Self::setting_row(
                "settings.window_transparency",
                Some("settings.window_transparency_desc"),
                Self::percent_slider(&self.settings_modal.window_transparency, transparency, p),
                p,
            ))
            .into_any_element()
    }

    /// 百分比滑块控件：滑轨加右侧数值。
    fn percent_slider(state: &Entity<SliderState>, percent: u8, p: &Palette) -> AnyElement {
        let label = format!("{percent}%");
        h_flex()
            .gap_2()
            .items_center()
            .child(Slider::new(state).w(px(160.0)))
            .child(div().text_xs().text_color(p.muted_foreground).child(label))
            .into_any_element()
    }

    /// 窗口整体透明度变化：写进设置并刷新整窗外观。
    pub(crate) fn set_window_transparency(
        &mut self,
        percent: u8,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let percent = percent.min(100);
        if self.settings.window.window_transparency == percent {
            return;
        }
        self.settings.window.window_transparency = percent;
        self.refresh_window_appearance(window, cx);
    }

    /// 窗口背景模式变化：切到半透明或毛玻璃时若透明度仍是 0，给一个看得出来的
    /// 初值，否则用户只看到模式变了、界面没变。
    pub(crate) fn set_window_background_mode(
        &mut self,
        mode: WindowBackgroundMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.settings.window.window_background == mode {
            return;
        }
        self.settings.window.window_background = mode;
        if mode != WindowBackgroundMode::Opaque && self.settings.window.window_transparency == 0 {
            self.settings.window.window_transparency = DEFAULT_WINDOW_TRANSPARENCY;
            let value = f32::from(DEFAULT_WINDOW_TRANSPARENCY);
            self.settings_modal
                .window_transparency
                .update(cx, |state, cx| {
                    state.set_value(value, window, cx);
                });
        }
        self.refresh_window_appearance(window, cx);
    }

    /// 把窗口外观下发到已打开的窗格：终端背景随窗口整体透明。
    pub(crate) fn push_window_appearance(&self, cx: &mut Context<Self>) {
        self.apply_appearance_to_live_panes(cx);
    }

    /// 窗口外观变化后的统一落地：全局外观值、主题重算与窗口背景处理。
    pub(crate) fn refresh_window_appearance(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        crate::window_appearance::apply_window_appearance(&self.settings.window, cx);
        crate::theme::apply_appearance(&self.settings.appearance, cx);
        window.set_background_appearance(crate::window_appearance::background_appearance(
            &self.settings.window,
        ));
        cx.notify();
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
