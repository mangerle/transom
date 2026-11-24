//! RDP 桌面视口悬浮快捷操作条。
//!
//! 设计原理（Why）：远程桌面全屏或大窗格操作时，宿主系统的常规快捷键（如 Ctrl+Alt+Del、
//! Win 键）常被本地系统拦截无法直接传给远端；通过在桌面视口顶部提供紧凑的悬浮工具条，
//! 并使用统一主题调色板 [`Palette`] 与标准矢量图标，确保亮色/暗色主题下视觉层级一致。

use crate::palette::Palette;
use gpui_kit::base::{Selectable, h_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::*;
use transom_ui_icons::{AppIcon, AppIconElement, CustomIcon, UiIcon};

/// 工具条按钮点击回调函数类型。
pub type RdpToolbarCallback = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// 悬浮工具条渲染参数。
pub struct RdpToolbarParams<'a> {
    pub palette: &'a Palette,
    pub is_muted: bool,
    pub is_scaled: bool,
    pub is_fullscreen: bool,
    pub is_pinned: bool,
    pub rx_bytes_per_sec: u64,
    pub rtt_ms: Option<u32>,
    pub on_send_cad: RdpToolbarCallback,
    pub on_send_win: RdpToolbarCallback,
    pub on_toggle_mute: RdpToolbarCallback,
    pub on_toggle_scale: RdpToolbarCallback,
    pub on_toggle_fullscreen: RdpToolbarCallback,
    pub on_toggle_pin: RdpToolbarCallback,
    pub on_disconnect: RdpToolbarCallback,
}

fn toolbar_icon_button(
    id: &'static str,
    icon: AppIcon,
    tooltip: String,
    selected: bool,
    danger: bool,
    on_click: RdpToolbarCallback,
) -> Button {
    let mut btn = Button::new(ElementId::Name(id.into()))
        .small()
        .child(AppIconElement::new(icon).small())
        .tooltip(tooltip)
        .on_click(on_click);
    if danger {
        btn = btn.danger();
    } else {
        btn = btn.ghost().selected(selected);
    }
    btn
}

/// 渲染紧凑纯图标悬浮快捷操作条（鼠标悬停显示中文提示）。
pub fn render_rdp_toolbar(params: RdpToolbarParams<'_>) -> AnyElement {
    let p = params.palette;
    h_flex()
        .items_center()
        .gap_1()
        .p_1()
        .rounded(p.radius.md)
        .bg(p.surface.floating)
        .border_1()
        .border_color(p.border)
        .shadow_md()
        .child(toolbar_icon_button(
            "rdp-cad-btn",
            AppIcon::Ui(UiIcon::KEYBOARD),
            rust_i18n::t!("rdp.send_cad").to_string(),
            false,
            false,
            params.on_send_cad,
        ))
        .child(toolbar_icon_button(
            "rdp-win-btn",
            AppIcon::Ui(UiIcon::DESKTOP),
            rust_i18n::t!("rdp.send_win").to_string(),
            false,
            false,
            params.on_send_win,
        ))
        .child(toolbar_icon_button(
            "rdp-mute-btn",
            if params.is_muted {
                AppIcon::Custom(CustomIcon::VOLUME_OFF)
            } else {
                AppIcon::Custom(CustomIcon::VOLUME_ON)
            },
            if params.is_muted {
                rust_i18n::t!("rdp.muted_tooltip", status = rust_i18n::t!("rdp.muted")).to_string()
            } else {
                rust_i18n::t!(
                    "rdp.sound_on_tooltip",
                    status = rust_i18n::t!("rdp.sound_on")
                )
                .to_string()
            },
            !params.is_muted,
            false,
            params.on_toggle_mute,
        ))
        .child(toolbar_icon_button(
            "rdp-scale-btn",
            AppIcon::Ui(UiIcon::LAYOUT),
            if params.is_scaled {
                rust_i18n::t!(
                    "rdp.scale_to_fit_tooltip",
                    status = rust_i18n::t!("rdp.scale_to_fit")
                )
                .to_string()
            } else {
                rust_i18n::t!(
                    "rdp.original_scale_tooltip",
                    status = rust_i18n::t!("rdp.original_scale")
                )
                .to_string()
            },
            params.is_scaled,
            false,
            params.on_toggle_scale,
        ))
        .child(toolbar_icon_button(
            "rdp-fullscreen-btn",
            if params.is_fullscreen {
                AppIcon::Custom(CustomIcon::FULLSCREEN_EXIT)
            } else {
                AppIcon::Custom(CustomIcon::FULLSCREEN)
            },
            if params.is_fullscreen {
                rust_i18n::t!("rdp.exit_fullscreen").to_string()
            } else {
                rust_i18n::t!("rdp.fullscreen").to_string()
            },
            params.is_fullscreen,
            false,
            params.on_toggle_fullscreen,
        ))
        .child(toolbar_icon_button(
            "rdp-pin-btn",
            AppIcon::Ui(UiIcon::LOCKED),
            if params.is_pinned {
                rust_i18n::t!("rdp.unpin_toolbar").to_string()
            } else {
                rust_i18n::t!("rdp.pin_toolbar").to_string()
            },
            params.is_pinned,
            false,
            params.on_toggle_pin,
        ))
        .child(render_network_telemetry_badge(
            params.rx_bytes_per_sec,
            params.rtt_ms,
            p,
        ))
        .child(toolbar_icon_button(
            "rdp-disconnect-btn",
            AppIcon::Ui(UiIcon::STOP),
            rust_i18n::t!("rdp.disconnect").to_string(),
            false,
            true,
            params.on_disconnect,
        ))
        .into_any_element()
}

/// 渲染紧凑的实时网络延时（RTT）与下行速率胶囊，鼠标悬停显示中文说明。
fn render_network_telemetry_badge(
    rx_bytes_per_sec: u64,
    rtt_ms: Option<u32>,
    p: &Palette,
) -> AnyElement {
    let rtt_text = rtt_ms.map_or_else(|| "-- ms".to_string(), |ms| format!("{ms} ms"));
    let rate_text = format_bytes_per_sec(rx_bytes_per_sec);
    let rtt_color = match rtt_ms {
        Some(ms) if ms >= 150 => p.warning,
        _ => p.muted_foreground,
    };

    div()
        .id("rdp-network-telemetry")
        .px_2()
        .py_0p5()
        .rounded(p.radius.sm)
        .bg(p.muted)
        .text_xs()
        .text_color(rtt_color)
        .child(format!("{rtt_text} · {rate_text}"))
        .tooltip(|window, cx| {
            Tooltip::new(rust_i18n::t!("rdp.network_telemetry_tooltip").to_string())
                .build(window, cx)
        })
        .into_any_element()
}

fn format_bytes_per_sec(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB/s", (bytes as f64) / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{} KB/s", bytes / 1024)
    } else {
        format!("{bytes} B/s")
    }
}
