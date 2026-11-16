//! 应用级界面语言设置与快捷切换。
//!
//! 设计原理（Why）：语言切换作用于全局设置持久化与 i18n locale，不属于标签关闭或
//! 任何特定工作区子域，因此独立归属到 `app/` 模块。

use super::{AppShell, AppState, app_settings, update_app_settings};
use crate::palette::localized_error;
use gpui_kit::component::set_locale;
use gpui_kit::*;

impl AppState {
    /// 设置界面语言并立即落盘；空串表示跟随系统。
    pub(crate) fn set_language(&mut self, language: String, cx: &mut Context<AppShell>) {
        if app_settings(cx).language == language {
            return;
        }
        update_app_settings(cx, |s| s.language = language);
        set_locale(&crate::i18n::resolve_language(&app_settings(cx).language));
        let settings_snapshot = app_settings(cx).clone();
        if let Err(error) = self.persist_settings(&settings_snapshot) {
            self.set_error(localized_error(&error));
        }
        cx.notify();
    }

    /// 活动栏的语言按钮：在中文与英文之间切换（跟随系统时以当前生效语言为起点）。
    pub(crate) fn toggle_language(&mut self, cx: &mut Context<AppShell>) {
        let next = if crate::i18n::resolve_language(&app_settings(cx).language) == "zh-CN" {
            "en"
        } else {
            "zh-CN"
        };
        self.set_language(next.to_string(), cx);
    }
}
