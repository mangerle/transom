//! 偏好设置中心之 Git 版本控制配置页（对标 openidea / IntelliJ IDEA 设置体系）。
//!
//! 设计原理（Why）：
//! - 将版本控制相关偏好独立归档，集中管理 Git 可执行程序路径、提交推送安全与差异对比偏好；
//! - 遵循 GPUI-kit 与 Transom 统一设置组件规范（`setting_row`, `setting_field_row`）；
//! - 各输入字段直接绑定 `SettingsModalState`，由 `collect_git_preferences` 统一校验与落盘。

use crate::app::{app_settings, update_app_settings};
use crate::palette::Palette;
use crate::settings::SettingsView;
use gpui_kit::base::v_flex;
use gpui_kit::component::Sizable;
use gpui_kit::component::switch::Switch;
use gpui_kit::*;

impl SettingsView {
    /// Git 可执行程序与基础环境组。
    pub(crate) fn render_settings_git_executable(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        v_flex()
            .gap_2()
            .child(crate::widgets::setting_field_row(
                crate::widgets::form_field(
                    rust_i18n::t!("settings.git_executable_path").to_string(),
                    self.state.git_executable_path.clone(),
                    p,
                ),
                Some(rust_i18n::t!("settings.git_executable_path_desc").to_string()),
                p,
            ))
            .child(crate::widgets::setting_row(
                "settings.git_auto_update_on_focus",
                Some("settings.git_auto_update_on_focus_desc"),
                Switch::new("toggle-git-auto-update")
                    .small()
                    .checked(app_settings(cx).git.auto_update_on_focus)
                    .on_click(cx.listener(|_this, &checked, _, cx| {
                        update_app_settings(cx, |s| s.git.auto_update_on_focus = checked);
                        cx.notify();
                    }))
                    .into_any_element(),
                p,
            ))
            .into_any_element()
    }

    /// 提交与推送安全组（提交选项、推送弹窗与保护分支）。
    pub(crate) fn render_settings_git_commit_push(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        v_flex()
            .gap_2()
            .child(crate::widgets::setting_row(
                "settings.git_push_dialog",
                Some("settings.git_push_dialog_desc"),
                Switch::new("toggle-git-push-dialog")
                    .small()
                    .checked(app_settings(cx).git.push_dialog)
                    .on_click(cx.listener(|_this, &checked, _, cx| {
                        update_app_settings(cx, |s| s.git.push_dialog = checked);
                        cx.notify();
                    }))
                    .into_any_element(),
                p,
            ))
            .child(crate::widgets::setting_row(
                "settings.git_clear_message",
                Some("settings.git_clear_message_desc"),
                Switch::new("toggle-git-clear-message")
                    .small()
                    .checked(app_settings(cx).git.clear_message_on_commit)
                    .on_click(cx.listener(|_this, &checked, _, cx| {
                        update_app_settings(cx, |s| s.git.clear_message_on_commit = checked);
                        cx.notify();
                    }))
                    .into_any_element(),
                p,
            ))
            .child(crate::widgets::setting_field_row(
                crate::widgets::form_field(
                    rust_i18n::t!("settings.git_protected_branches").to_string(),
                    self.state.git_protected_branches.clone(),
                    p,
                ),
                Some(rust_i18n::t!("settings.git_protected_branches_desc").to_string()),
                p,
            ))
            .child(crate::widgets::setting_field_row(
                crate::widgets::form_field(
                    rust_i18n::t!("settings.git_author_name").to_string(),
                    self.state.git_author_name.clone(),
                    p,
                ),
                Some(rust_i18n::t!("settings.git_author_name_desc").to_string()),
                p,
            ))
            .child(crate::widgets::setting_field_row(
                crate::widgets::form_field(
                    rust_i18n::t!("settings.git_author_email").to_string(),
                    self.state.git_author_email.clone(),
                    p,
                ),
                Some(rust_i18n::t!("settings.git_author_email_desc").to_string()),
                p,
            ))
            .into_any_element()
    }

    /// 差异对比偏好组（空白忽略与默认视图）。
    pub(crate) fn render_settings_git_diff(
        &mut self,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        v_flex()
            .gap_2()
            .child(crate::widgets::setting_row(
                "settings.git_ignore_whitespace",
                Some("settings.git_ignore_whitespace_desc"),
                Switch::new("toggle-git-ignore-whitespace")
                    .small()
                    .checked(app_settings(cx).git.ignore_whitespace)
                    .on_click(cx.listener(|_this, &checked, _, cx| {
                        update_app_settings(cx, |s| s.git.ignore_whitespace = checked);
                        cx.notify();
                    }))
                    .into_any_element(),
                p,
            ))
            .into_any_element()
    }
}
