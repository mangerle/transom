//! 偏好设置中心之鼠标、选区与剪贴板配置页渲染。

use crate::app::AppShell;
use crate::app::app_settings;
use crate::app::app_settings_mut;
use crate::app::update_app_settings;
use crate::palette::Palette;
use gpui_kit::base::v_flex;
use gpui_kit::component::Sizable;
use gpui_kit::component::switch::Switch;
use gpui_kit::*;

impl AppShell {
    /// 鼠标组：右键行为与中键粘贴开关。
    pub(crate) fn render_settings_mouse(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let behavior = app_settings(cx).terminal_interaction.right_click_behavior();
        let behavior_options = vec![
            (
                transom_core::RightClickBehavior::Paste,
                rust_i18n::t!("settings.right_click_paste_action").to_string(),
            ),
            (
                transom_core::RightClickBehavior::Menu,
                rust_i18n::t!("settings.right_click_menu").to_string(),
            ),
            (
                transom_core::RightClickBehavior::MenuWithPaste,
                rust_i18n::t!("settings.right_click_menu_paste").to_string(),
            ),
        ];
        let rc_dropdown = Self::setting_dropdown(
            "dropdown-right-click-action",
            behavior,
            behavior_options,
            |_this, new_behavior, cx| {
                app_settings_mut(cx)
                    .terminal_interaction
                    .set_right_click_behavior(new_behavior);
                cx.notify();
            },
            cx,
        );

        v_flex()
            .gap_2()
            .child(crate::widgets::setting_row(
                "settings.right_click_action",
                Some("settings.right_click_action_desc"),
                rc_dropdown,
                p,
            ))
            .child(crate::widgets::setting_row(
                "settings.middle_click_paste",
                Some("settings.middle_click_paste_desc"),
                Switch::new("toggle-middle-click-paste")
                    .small()
                    .checked(app_settings(cx).terminal_interaction.middle_click_paste)
                    .on_click(cx.listener(|_this, &checked, _, cx| {
                        update_app_settings(cx, |s| {
                            s.terminal_interaction.middle_click_paste = checked
                        });
                        cx.notify();
                    }))
                    .into_any_element(),
                p,
            ))
            .into_any_element()
    }

    /// 选择组：双击分词边界。
    pub(crate) fn render_settings_selection(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        v_flex()
            .gap_2()
            .child(crate::widgets::setting_field_row(
                crate::widgets::form_field(
                    rust_i18n::t!("settings.word_separators").to_string(),
                    self.settings_modal(cx).word_separators.clone(),
                    p,
                ),
                Some(rust_i18n::t!("settings.word_separators_note").to_string()),
                p,
            ))
            .into_any_element()
    }

    /// 剪贴板组：选中复制、多行粘贴确认、尾随空白清理与拦截误按的 Ctrl+V。
    pub(crate) fn render_settings_clipboard(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        v_flex()
            .gap_2()
            .child(crate::widgets::setting_row(
                "settings.copy_on_select",
                Some("settings.copy_on_select_desc"),
                Switch::new("toggle-copy-on-select")
                    .small()
                    .checked(app_settings(cx).terminal_interaction.copy_on_select)
                    .on_click(cx.listener(|_this, &checked, _, cx| {
                        update_app_settings(cx, |s| {
                            s.terminal_interaction.copy_on_select = checked
                        });
                        cx.notify();
                    }))
                    .into_any_element(),
                p,
            ))
            .child(crate::widgets::setting_row(
                "settings.multiline_paste",
                Some("settings.multiline_paste_desc"),
                Switch::new("toggle-multiline-paste")
                    .small()
                    .checked(
                        app_settings(cx)
                            .terminal_interaction
                            .multiline_paste_confirm,
                    )
                    .on_click(cx.listener(|_this, &checked, _, cx| {
                        app_settings_mut(cx)
                            .terminal_interaction
                            .multiline_paste_confirm = checked;
                        cx.notify();
                    }))
                    .into_any_element(),
                p,
            ))
            .child(crate::widgets::setting_row(
                "settings.trim_trailing_whitespace",
                Some("settings.trim_trailing_whitespace_desc"),
                Switch::new("toggle-trim-whitespace")
                    .small()
                    .checked(
                        app_settings(cx)
                            .terminal_interaction
                            .trim_trailing_whitespace,
                    )
                    .on_click(cx.listener(|_this, &checked, _, cx| {
                        app_settings_mut(cx)
                            .terminal_interaction
                            .trim_trailing_whitespace = checked;
                        cx.notify();
                    }))
                    .into_any_element(),
                p,
            ))
            .child(crate::widgets::setting_row(
                "settings.suppress_ctrl_v",
                Some("settings.suppress_ctrl_v_desc"),
                Switch::new("toggle-suppress-ctrl-v")
                    .small()
                    .checked(app_settings(cx).terminal_interaction.suppress_ctrl_v)
                    .on_click(cx.listener(|_this, &checked, _, cx| {
                        update_app_settings(cx, |s| {
                            s.terminal_interaction.suppress_ctrl_v = checked
                        });
                        cx.notify();
                    }))
                    .into_any_element(),
                p,
            ))
            .into_any_element()
    }
}
