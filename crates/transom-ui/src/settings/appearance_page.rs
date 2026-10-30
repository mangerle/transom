//! 偏好设置中心之界面主题、明暗模式与密度档位配置页渲染。

use super::{FontFieldSpec, slider_percent};
use crate::app::AppShell;
use crate::divider::{DividerEdge, DividerStyled};
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
use transom_core::{
    DividerColor, DividerWidth, PaletteMode, UiDensity, UiThemeMode, WindowBackgroundMode,
    preset_themes,
};

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
            .w_full()
            .justify_between()
            .items_center()
            .py_2()
            .border_color(p.divider(p.border))
            .divider_edge(DividerEdge::Bottom, p.divider_width)
            .child(
                div()
                    .text_sm()
                    .child(rust_i18n::t!("settings.ui_mode").to_string()),
            )
            .child(Self::setting_dropdown(
                "dropdown-ui-mode",
                self.settings.appearance.mode,
                vec![
                    (
                        UiThemeMode::System,
                        rust_i18n::t!("settings.ui_mode_system").to_string(),
                    ),
                    (
                        UiThemeMode::Light,
                        rust_i18n::t!("settings.ui_mode_light").to_string(),
                    ),
                    (
                        UiThemeMode::Dark,
                        rust_i18n::t!("settings.ui_mode_dark").to_string(),
                    ),
                ],
                |this, mode, cx| {
                    this.update_appearance(|appearance| appearance.mode = mode, cx);
                },
                cx,
            ));

        let density_row = h_flex()
            .w_full()
            .justify_between()
            .items_center()
            .py_2()
            .border_color(p.divider(p.border))
            .divider_edge(DividerEdge::Bottom, p.divider_width)
            .child(
                div()
                    .text_sm()
                    .child(rust_i18n::t!("settings.density").to_string()),
            )
            .child(Self::setting_dropdown(
                "dropdown-ui-density",
                self.settings.appearance.density,
                UiDensity::ALL
                    .into_iter()
                    .map(|d| (d, rust_i18n::t!(d.label_key()).to_string()))
                    .collect(),
                |this, density, cx| {
                    this.update_appearance(|appearance| appearance.density = density, cx);
                },
                cx,
            ));

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

        // 主题卡自带边框，不按行画线；说明、卡片与反差提示同属一项，在组末补一条。
        let card_block =
            v_flex()
                .w_full()
                .gap_2()
                .child(
                    div()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(rust_i18n::t!("settings.ui_theme_note").to_string()),
                )
                .children(cards)
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
                                div().flex_1().text_xs().text_color(p.warning).child(
                                    rust_i18n::t!("settings.appearance_contrast").to_string(),
                                ),
                            ),
                    )
                });

        v_flex()
            .gap_3()
            .child(mode_row)
            .child(density_row)
            .child(Self::setting_frame(card_block.into_any_element(), p))
            .into_any_element()
    }

    /// 界面字体组：界面字体族覆盖；字号由配色方案页的密度档驱动。
    pub(crate) fn render_settings_ui_font(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        v_flex()
            .gap_2()
            .child(Self::setting_font_field(
                FontFieldSpec {
                    id: "ui-font-family",
                    label: rust_i18n::t!("settings.ui_font_family").to_string(),
                    select: self.modals.settings.ui_font_family.clone(),
                    note_key: Some("settings.ui_font_note"),
                },
                p,
                cx,
            ))
            .into_any_element()
    }

    /// 分割线组：颜色语义与粗细；改一档经 [`AppShell::update_appearance`] 立即
    /// 全局生效，保存/取消分别负责落盘与回滚。
    pub(crate) fn render_settings_divider(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let divider = self.settings.appearance.divider;
        // 预览色点与真实渲染走同一条解析：跟随主题时显示当前主题边框色。
        let preview = p.divider(p.border);
        let color_control = h_flex()
            .gap_2()
            .items_center()
            .child(
                div()
                    .size(px(12.0))
                    .rounded_full()
                    .bg(preview)
                    .border_1()
                    .border_color(p.border),
            )
            .child(Self::setting_dropdown(
                "dropdown-divider-color",
                divider.color,
                [
                    DividerColor::Theme,
                    DividerColor::Border,
                    DividerColor::Muted,
                    DividerColor::Foreground,
                    DividerColor::Primary,
                ]
                .into_iter()
                .map(|color| (color, rust_i18n::t!(color.label_key()).to_string()))
                .collect(),
                |this, color, cx| {
                    this.update_appearance(|appearance| appearance.divider.color = color, cx);
                },
                cx,
            ))
            .into_any_element();
        // 粗细是 1/2/3 的连续小数值：步进器比下拉少一次点击，和字号/行高一致；
        // 量纲写在说明文案里，输入框内只留数字，避免控件被单位挤窄。
        let width_control = Self::setting_stepper(&self.modals.settings.divider_width, px(110.0));
        v_flex()
            .gap_2()
            .child(Self::setting_row(
                "settings.divider_color",
                Some("settings.divider_color_desc"),
                color_control,
                p,
            ))
            .child(Self::setting_row(
                "settings.divider_width",
                Some("settings.divider_width_desc"),
                width_control,
                p,
            ))
            .into_any_element()
    }

    /// 分割线粗细步进器变化：解析为档位后立即全局预览。
    ///
    /// 与颜色下拉共用 [`AppShell::update_appearance`] 这条写路径，保存/取消的
    /// 落盘与回滚语义因此完全一致；输到一半的非法数字（空值、越界）不落地，
    /// 避免界面闪一帧错误线宽。
    pub(crate) fn apply_divider_width_from_input(&mut self, cx: &mut Context<Self>) {
        let value = self
            .modals
            .settings
            .divider_width
            .read(cx)
            .value()
            .to_string();
        let Some(width) = divider_width_from_value(&value) else {
            return;
        };
        if self.settings.appearance.divider.width == width {
            return;
        }
        self.update_appearance(|appearance| appearance.divider.width = width, cx);
    }

    /// 窗口外观组：背景模式与整体透明度。
    pub(crate) fn render_settings_window_appearance(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mode = self.settings.window.window_background;
        let transparency =
            slider_percent(&self.modals.settings.window_transparency.read(cx).value());
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
                Self::percent_slider(&self.modals.settings.window_transparency, transparency, p),
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
            self.modals
                .settings
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

/// 步进器数值 → 分割线档位：只认 1..=3 的整数值，其余输入一律不生效。
///
/// 输入框允许用户敲出任意数字（含删空重输的中间态与小数点），先把文本收敛
/// 成枚举再写入偏好，运行期与落盘侧拿到的就永远是合法档位，不需要调用方再
/// 校验；`2.0` 这类整数值照常接受，`2.5`、越界与脏字符则保持原档位。
fn divider_width_from_value(value: &str) -> Option<DividerWidth> {
    let pixels = value.trim().parse::<f64>().ok()?;
    if pixels.fract() != 0.0 {
        return None;
    }
    match pixels as i32 {
        1 => Some(DividerWidth::Hairline),
        2 => Some(DividerWidth::Medium),
        3 => Some(DividerWidth::Thick),
        _ => None,
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

#[cfg(test)]
mod tests {
    use super::divider_width_from_value;
    use transom_core::DividerWidth;

    #[test]
    fn divider_width_value_maps_to_three_steps() {
        assert_eq!(divider_width_from_value("1"), Some(DividerWidth::Hairline));
        assert_eq!(divider_width_from_value(" 2 "), Some(DividerWidth::Medium));
        assert_eq!(divider_width_from_value("3"), Some(DividerWidth::Thick));
    }

    #[test]
    fn divider_width_value_accepts_integer_like_decimals() {
        assert_eq!(divider_width_from_value("2.0"), Some(DividerWidth::Medium));
        assert_eq!(divider_width_from_value("3.00"), Some(DividerWidth::Thick));
    }

    #[test]
    fn divider_width_value_rejects_out_of_range_and_garbage() {
        assert_eq!(divider_width_from_value("0"), None);
        assert_eq!(divider_width_from_value("4"), None);
        assert_eq!(divider_width_from_value(""), None);
        assert_eq!(divider_width_from_value("2.5"), None);
        assert_eq!(divider_width_from_value("2px"), None);
    }
}
