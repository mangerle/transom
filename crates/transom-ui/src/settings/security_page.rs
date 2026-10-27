//! 偏好设置中心之历史记录与安全审计配置页渲染。

use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::Button;
use gpui_kit::component::switch::Switch;
use gpui_kit::*;

impl AppShell {
    /// 历史组：命令采集与建议开关。
    pub(crate) fn render_settings_history(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        v_flex()
            .gap_2()
            .child(Self::setting_row(
                "settings.history_enabled",
                Some("settings.history_enabled_desc"),
                Switch::new("toggle-history-enabled")
                    .small()
                    .checked(self.settings.history.enabled)
                    .on_click(cx.listener(|this, &checked, _, cx| {
                        this.settings.history.enabled = checked;
                        // 关掉时把内存里的建议一起收起：下次开回来仍从文件恢复，
                        // 不做删除，避免"关一下就把历史清空"的误伤。
                        if !checked {
                            this.history.write().clear();
                        } else if this.history.read().is_empty() {
                            let loaded = this.services.history.load();
                            *this.history.write() = loaded;
                        }
                        cx.notify();
                    }))
                    .into_any_element(),
                p,
            ))
            .into_any_element()
    }

    /// 审计组：会话输出落盘开关与日志目录入口。
    pub(crate) fn render_settings_audit(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let dir = transom_core::paths::audit_dir();
        let reveal_dir = dir.clone();
        let reveal = cx.listener(move |this, _, _, cx| {
            crate::audit::reveal_in_file_manager(reveal_dir.clone());
            this.status = Some(rust_i18n::t!("settings.audit_revealed").to_string());
            cx.notify();
        });
        v_flex()
            .gap_2()
            .child(Self::setting_row(
                "settings.audit_enabled",
                Some("settings.audit_enabled_desc"),
                Switch::new("toggle-audit-enabled")
                    .small()
                    .checked(self.settings.audit.enabled)
                    .on_click(cx.listener(|this, &checked, _, cx| {
                        this.settings.audit.enabled = checked;
                        cx.notify();
                    }))
                    .into_any_element(),
                p,
            ))
            .child(Self::setting_row(
                "settings.audit_dir",
                Some("settings.audit_dir_desc"),
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(Self::setting_path_value(
                        "audit-dir",
                        dir.display().to_string(),
                        p,
                    ))
                    .child(
                        Button::new("audit-viewer-open")
                            .small()
                            .outline()
                            .label(rust_i18n::t!("audit_viewer.title").to_string())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_audit_viewer(None, window, cx);
                            })),
                    )
                    .child(
                        Button::new("audit-dir-reveal")
                            .small()
                            .label(rust_i18n::t!("settings.audit_dir_open").to_string())
                            .on_click(reveal),
                    )
                    .into_any_element(),
                p,
            ))
            .into_any_element()
    }
}
