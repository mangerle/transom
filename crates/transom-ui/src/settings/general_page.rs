//! 偏好设置中的通用与声音页。

use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{Selectable, h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::Button;
use gpui_kit::component::switch::Switch;
use gpui_kit::*;
use transom_core::BellStyle;

impl AppShell {
    pub(crate) fn render_settings_general(
        &mut self,
        _window: &mut Window,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let current_bell = self.settings.window.bell_style;

        v_flex()
            .gap_4()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::BOLD)
                    .child(rust_i18n::t!("settings.general").to_string()),
            )
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .py_1()
                    .child(
                        div()
                            .text_sm()
                            .child(rust_i18n::t!("app.language").to_string()),
                    )
                    .child(
                        Button::new("settings-lang")
                            .small()
                            .label(if self.settings.language == "zh-CN" {
                                "中文"
                            } else {
                                "English"
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.toggle_language(cx);
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
                            .child(rust_i18n::t!("settings.bell_style").to_string()),
                    )
                    .child(
                        h_flex().gap_1().children(
                            [
                                ("bell-none", "settings.bell_none", BellStyle::None),
                                ("bell-visual", "settings.bell_visual", BellStyle::Visual),
                                ("bell-audible", "settings.bell_audible", BellStyle::Audible),
                            ]
                            .into_iter()
                            .map(|(id, key, style)| {
                                Button::new(id)
                                    .small()
                                    .label(rust_i18n::t!(key).to_string())
                                    .selected(current_bell == style)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.settings.window.bell_style = style;
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
                            .child(rust_i18n::t!("notify.desktop_switch").to_string()),
                    )
                    .child(
                        Switch::new("toggle-desktop-notifications")
                            .checked(self.settings.window.desktop_notifications)
                            .on_click(cx.listener(|this, &checked, _, cx| {
                                this.settings.window.desktop_notifications = checked;
                                cx.notify();
                            })),
                    ),
            )
            .child(Self::form_field(
                rust_i18n::t!("notify.threshold").to_string(),
                self.settings_notify_threshold.clone(),
                p,
            ))
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("notify.threshold_note").to_string()),
            )
            .child(
                v_flex()
                    .gap_2()
                    .pt_2()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::BOLD)
                            .child(rust_i18n::t!("backup.title").to_string()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child(rust_i18n::t!("backup.settings_hint").to_string()),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("backup-export")
                                    .small()
                                    .icon(IconName::Download)
                                    .label(rust_i18n::t!("backup.export_action").to_string())
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.open_backup_export(window, cx);
                                    })),
                            )
                            .child(
                                Button::new("backup-import")
                                    .small()
                                    .icon(IconName::Upload)
                                    .label(rust_i18n::t!("backup.import_action").to_string())
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.open_backup_import(window, cx);
                                    })),
                            ),
                    ),
            )
            .into_any_element()
    }
}
