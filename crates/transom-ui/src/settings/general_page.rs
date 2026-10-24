//! 偏好设置中的通用页：语言、通知与声音、数据备份。

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
    /// 语言组：界面语言切换。
    pub(crate) fn render_settings_language(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        v_flex()
            .gap_2()
            .child(Self::setting_row(
                "app.language",
                Some("settings.language_desc"),
                Button::new("settings-lang")
                    .small()
                    .label(if self.settings.language == "zh-CN" {
                        "中文"
                    } else {
                        "English"
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.toggle_language(cx);
                    }))
                    .into_any_element(),
                p,
            ))
            .into_any_element()
    }

    /// 通知与声音组：终端响铃、桌面通知与通知阈值。
    pub(crate) fn render_settings_notify(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let current_bell = self.settings.window.bell_style;
        v_flex()
            .gap_2()
            .child(Self::setting_row(
                "settings.bell_style",
                Some("settings.bell_style_desc"),
                h_flex()
                    .gap_1()
                    .children(
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
                    )
                    .into_any_element(),
                p,
            ))
            .child(Self::setting_row(
                "notify.desktop_switch",
                Some("settings.desktop_notifications_desc"),
                Switch::new("toggle-desktop-notifications")
                    .checked(self.settings.window.desktop_notifications)
                    .on_click(cx.listener(|this, &checked, _, cx| {
                        this.settings.window.desktop_notifications = checked;
                        cx.notify();
                    }))
                    .into_any_element(),
                p,
            ))
            .child(Self::form_field(
                rust_i18n::t!("notify.threshold").to_string(),
                self.settings_modal.notify_threshold.clone(),
                p,
            ))
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("notify.threshold_note").to_string()),
            )
            .into_any_element()
    }

    /// 数据备份组：全量导入与导出。
    pub(crate) fn render_settings_backup(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        v_flex()
            .gap_2()
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
            )
            .into_any_element()
    }
}
