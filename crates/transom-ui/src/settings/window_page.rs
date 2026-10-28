//! 偏好设置中心之标签页、面板停靠与窗口记忆配置页渲染。

use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::base::v_flex;
use gpui_kit::component::Sizable;
use gpui_kit::component::switch::Switch;
use gpui_kit::*;
use transom_core::PanelSide;

impl AppShell {
    /// 标签页组：关闭按钮与最后标签退出行为。
    pub(crate) fn render_settings_tabs(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        v_flex()
            .gap_2()
            .child(Self::setting_row(
                "settings.show_tab_close_button",
                Some("settings.show_tab_close_button_desc"),
                Switch::new("toggle-tab-close-btn")
                    .small()
                    .checked(self.settings.window.show_tab_close_button)
                    .on_click(cx.listener(|this, &checked, _, cx| {
                        this.settings.window.show_tab_close_button = checked;
                        cx.notify();
                    }))
                    .into_any_element(),
                p,
            ))
            .child(Self::setting_row(
                "settings.close_window_on_all_tabs_closed",
                Some("settings.close_window_on_all_tabs_closed_desc"),
                Switch::new("toggle-close-on-last-tab")
                    .small()
                    .checked(self.settings.window.close_window_on_all_tabs_closed)
                    .on_click(cx.listener(|this, &checked, _, cx| {
                        this.settings.window.close_window_on_all_tabs_closed = checked;
                        cx.notify();
                    }))
                    .into_any_element(),
                p,
            ))
            .into_any_element()
    }

    /// 窗口组：停靠方向、窗口记忆与会话恢复。
    pub(crate) fn render_settings_window(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let side = self.settings.window.sftp_panel_side;
        v_flex()
            .gap_2()
            .child(Self::setting_row(
                "settings.sftp_panel_side",
                Some("settings.sftp_panel_side_desc"),
                Self::setting_dropdown(
                    "dropdown-sftp-panel-side",
                    side,
                    vec![
                        (
                            PanelSide::Left,
                            rust_i18n::t!("settings.panel_side_left").to_string(),
                        ),
                        (
                            PanelSide::Right,
                            rust_i18n::t!("settings.panel_side_right").to_string(),
                        ),
                    ],
                    |this, new_side, cx| {
                        this.set_sftp_panel_side(new_side, cx);
                    },
                    cx,
                ),
                p,
            ))
            .child(Self::setting_row(
                "settings.remember_window_bounds",
                Some("settings.remember_window_bounds_desc"),
                Switch::new("toggle-remember-bounds")
                    .small()
                    .checked(self.settings.window.remember_window_bounds)
                    .on_click(cx.listener(|this, &checked, _, cx| {
                        this.settings.window.remember_window_bounds = checked;
                        cx.notify();
                    }))
                    .into_any_element(),
                p,
            ))
            .into_any_element()
    }
}
