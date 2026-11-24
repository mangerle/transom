//! RDP 桌面视口悬浮快捷操作条。
//!
//! 设计原理（Why）：远程桌面全屏或大窗格操作时，宿主系统的常规快捷键（如 Ctrl+Alt+Del、
//! Win 键）常被本地系统拦截无法直接传给远端；通过在桌面视口顶部提供紧凑的悬浮工具条，
//! 并使用统一主题调色板 [`Palette`] 与标准矢量图标，确保亮色/暗色主题下视觉层级一致。

use crate::palette::Palette;
use gpui_kit::base::h_flex;
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::*;
use transom_ui_icons::{AppIcon, AppIconElement, UiIcon};

/// 工具条按钮点击回调函数类型。
pub type RdpToolbarCallback = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// 悬浮工具条渲染参数。
pub struct RdpToolbarParams<'a> {
    pub palette: &'a Palette,
    pub is_muted: bool,
    pub is_scaled: bool,
    pub on_send_cad: RdpToolbarCallback,
    pub on_send_win: RdpToolbarCallback,
    pub on_toggle_mute: RdpToolbarCallback,
    pub on_toggle_scale: RdpToolbarCallback,
    pub on_disconnect: RdpToolbarCallback,
}

/// 渲染悬浮快捷操作条。
pub fn render_rdp_toolbar(params: RdpToolbarParams<'_>) -> AnyElement {
    let p = params.palette;
    h_flex()
        .gap_1p5()
        .p_1()
        .rounded(p.radius.md)
        .bg(p.surface.floating)
        .border_1()
        .border_color(p.border)
        .child(
            Button::new(ElementId::Name("rdp-cad-btn".into()))
                .small()
                .ghost()
                .child(AppIconElement::new(AppIcon::Ui(UiIcon::KEYBOARD)).small())
                .label("Ctrl+Alt+Del")
                .on_click(params.on_send_cad),
        )
        .child(
            Button::new(ElementId::Name("rdp-win-btn".into()))
                .small()
                .ghost()
                .child(AppIconElement::new(AppIcon::Ui(UiIcon::DESKTOP)).small())
                .label("Win")
                .on_click(params.on_send_win),
        )
        .child(
            Button::new(ElementId::Name("rdp-mute-btn".into()))
                .small()
                .ghost()
                .label(if params.is_muted {
                    rust_i18n::t!("rdp.muted").to_string()
                } else {
                    rust_i18n::t!("rdp.sound_on").to_string()
                })
                .on_click(params.on_toggle_mute),
        )
        .child(
            Button::new(ElementId::Name("rdp-scale-btn".into()))
                .small()
                .ghost()
                .child(AppIconElement::new(AppIcon::Ui(UiIcon::MAXIMIZE)).small())
                .label(if params.is_scaled {
                    rust_i18n::t!("rdp.scale_to_fit").to_string()
                } else {
                    rust_i18n::t!("rdp.original_scale").to_string()
                })
                .on_click(params.on_toggle_scale),
        )
        .child(
            Button::new(ElementId::Name("rdp-disconnect-btn".into()))
                .small()
                .danger()
                .child(AppIconElement::new(AppIcon::Ui(UiIcon::STOP)).small())
                .label(rust_i18n::t!("rdp.disconnect").to_string())
                .on_click(params.on_disconnect),
        )
        .into_any_element()
}
