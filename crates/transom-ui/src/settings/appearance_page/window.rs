//! 窗口外观、透明度与分割线设置页。

use super::*;

impl AppShell {
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
            .child(crate::widgets::setting_row(
                "settings.divider_color",
                Some("settings.divider_color_desc"),
                color_control,
                p,
            ))
            .child(crate::widgets::setting_row(
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
            .child(crate::widgets::setting_row(
                "settings.window_background",
                Some("settings.window_background_desc"),
                mode_buttons,
                p,
            ))
            .child(crate::widgets::setting_row(
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
