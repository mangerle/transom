//! RDP 桌面视口悬浮快捷操作条。
//!
//! 设计原理（Why）：远程桌面全屏或大窗格操作时，宿主系统的常规快捷键（如 Ctrl+Alt+Del、
//! Win 键）常被本地系统拦截无法直接传给远端；通过在桌面视口顶部提供紧凑的半透明
//! 悬浮工具条，为用户提供一键发送系统组合键、声音静音、缩放模式切换与会话断开。

use gpui_kit::base::h_flex;
use gpui_kit::component::Sizable;
use gpui_kit::component::button::Button;
use gpui_kit::*;

/// 悬浮工具条渲染参数。
pub struct RdpToolbarParams {
    pub is_muted: bool,
    pub is_scaled: bool,
    pub on_send_cad: Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>,
    pub on_toggle_mute: Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>,
    pub on_toggle_scale: Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>,
    pub on_disconnect: Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>,
}

/// 渲染半透明悬浮快捷操作条。
pub fn render_rdp_toolbar(params: RdpToolbarParams) -> AnyElement {
    h_flex()
        .gap(px(6.0))
        .p(px(4.0))
        .rounded(px(6.0))
        .bg(gpui_kit::rgba(0x1e1e1ecc))
        .border_1()
        .border_color(gpui_kit::rgba(0xffffff22))
        .child(
            Button::new(ElementId::Name("rdp-cad-btn".into()))
                .small()
                .label("Ctrl+Alt+Del")
                .on_click(params.on_send_cad),
        )
        .child(
            Button::new(ElementId::Name("rdp-mute-btn".into()))
                .small()
                .label(if params.is_muted {
                    "静音中"
                } else {
                    "声音开启"
                })
                .on_click(params.on_toggle_mute),
        )
        .child(
            Button::new(ElementId::Name("rdp-scale-btn".into()))
                .small()
                .label(if params.is_scaled {
                    "自适应窗口"
                } else {
                    "原始比例"
                })
                .on_click(params.on_toggle_scale),
        )
        .child(
            Button::new(ElementId::Name("rdp-disconnect-btn".into()))
                .small()
                .label("断开连接")
                .on_click(params.on_disconnect),
        )
        .into_any_element()
}
