//! 用户主题的即时预览与注册入口。
//!
//! 预览仍走全局 `Theme` 的唯一写路径，但不写入 `appearance.ui_theme`：关闭设置
//! 或取消时，现有外观快照会重新套用旧主题。终端色板只在“跟随界面主题”模式下
//! 预览，显式选择的终端预设不会被编辑器悄悄覆盖。

use super::document::{ThemeDocumentError, ThemeDraft};
use super::{apply_surface_transparency, rebuild_terminal_bindings};
use crate::app::{app_settings, update_app_settings};
use gpui_kit::base::input::CursorBlink;
use gpui_kit::component::theme::{Theme, ThemeMode, ThemeRegistry};
use gpui_kit::{App, px};
use std::rc::Rc;
use std::time::Duration;
use transom_core::FOLLOW_UI_THEME;

/// 立即应用主题草稿，返回终端跟随色板是否也随之改变。
pub(crate) fn apply_theme_preview(draft: &ThemeDraft, cx: &mut App) -> bool {
    let appearance = app_settings(cx).appearance.clone();
    let density = appearance.density;
    let surface_alpha = crate::window_appearance::window_appearance(cx)
        .map(|appearance| appearance.surface_alpha)
        .unwrap_or(1.0);
    let config = Rc::new(draft.config.clone());
    let bundled_mono_font = config
        .mono_font_family
        .is_none()
        .then(|| crate::bundled_fonts::default_mono_font_family(cx))
        .flatten();
    Theme::update(cx, |theme| {
        theme.apply_config(&config);
        if config.mode == ThemeMode::Dark {
            theme.dark_theme = config.clone();
        } else {
            theme.light_theme = config.clone();
        }
        theme.font_size = px(density.font_size());
        theme.mono_font_size = px(density.mono_font_size());
        // 预览换装同样沿用空闲停止闪烁策略，避免退出预览后行为跳变。
        theme.cursor_blink = CursorBlink::default().with_idle_timeout(Duration::from_secs(5));
        if let Some(family) = bundled_mono_font {
            theme.mono_font_family = family;
        }
        if let Some(family) = appearance.ui_font_family.as_deref() {
            theme.font_family = family.into();
        }
        apply_surface_transparency(theme, surface_alpha);
    });

    draft.terminal_palette.as_ref().is_some_and(|palette| {
        if app_settings(cx).terminal_profile.theme_name != FOLLOW_UI_THEME {
            return false;
        }
        update_app_settings(cx, |settings| {
            settings.terminal_profile.palette = palette.clone();
        });
        true
    })
}

/// 把已保存到用户目录的主题立即装入注册表，无需等待目录监听回调。
pub(crate) fn register_theme_document(
    content: &str,
    cx: &mut App,
) -> Result<(), ThemeDocumentError> {
    ThemeRegistry::global_mut(cx)
        .load_themes_from_str(content)
        .map_err(|error| ThemeDocumentError::Registry {
            message: error.to_string(),
        })?;
    rebuild_terminal_bindings(cx);
    Ok(())
}
