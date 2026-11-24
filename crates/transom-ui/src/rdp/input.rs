//! RDP 界面交互输入事件转换。
//!
//! 设计原理（Why）：GPUI 界面层传递的坐标是带有窗格偏移、外边距与可能的
//! 缩放（FitWindow）的浮点逻辑像素；需要基于当前视口包围盒进行归一化并
//! 映射到远端桌面的实际物理像素坐标系，同时过滤掉视口外的越界坐标。

use gpui_kit::{Bounds, Pixels, Point};
use transom_rdp::{MouseBtn, RdpInput};

/// 计算鼠标在远端桌面上的物理像素坐标。
///
/// 优化说明：自适应缩放模式下采用严格保持宽高比的等比缩放，
/// 并按黑边居中偏移（Letterbox / Pillarbox）计算物理像素，杜绝视口拉伸形变与鼠标热点偏移。
pub fn map_pointer_position(
    cursor_pos: Point<Pixels>,
    viewport_bounds: Bounds<Pixels>,
    desktop_width: u16,
    desktop_height: u16,
    scale_to_fit: bool,
) -> Option<(u16, u16)> {
    if !viewport_bounds.contains(&cursor_pos) || desktop_width == 0 || desktop_height == 0 {
        return None;
    }

    let rel_x = f32::from(cursor_pos.x - viewport_bounds.origin.x);
    let rel_y = f32::from(cursor_pos.y - viewport_bounds.origin.y);

    let (scale, offset_x, offset_y) = if scale_to_fit {
        let view_w = f32::from(viewport_bounds.size.width);
        let view_h = f32::from(viewport_bounds.size.height);
        let s = (view_w / f32::from(desktop_width))
            .min(view_h / f32::from(desktop_height));
        let ox = (view_w - f32::from(desktop_width) * s) / 2.0;
        let oy = (view_h - f32::from(desktop_height) * s) / 2.0;
        (s, ox, oy)
    } else {
        (1.0, 0.0, 0.0)
    };

    if scale <= 0.0 {
        return None;
    }

    let desk_x = (rel_x - offset_x) / scale;
    let desk_y = (rel_y - offset_y) / scale;

    if desk_x < 0.0
        || desk_y < 0.0
        || desk_x >= f32::from(desktop_width)
        || desk_y >= f32::from(desktop_height)
    {
        return None;
    }

    let px = (desk_x.round() as u16).min(desktop_width - 1);
    let py = (desk_y.round() as u16).min(desktop_height - 1);
    Some((px, py))
}

/// 将 GPUI 鼠标按键转换为 RDP 输入指令。
pub fn map_mouse_button(button: gpui_kit::MouseButton, pressed: bool) -> Option<RdpInput> {
    let btn = match button {
        gpui_kit::MouseButton::Left => MouseBtn::Left,
        gpui_kit::MouseButton::Right => MouseBtn::Right,
        gpui_kit::MouseButton::Middle => MouseBtn::Middle,
        _ => return None,
    };
    Some(RdpInput::MouseButton {
        button: btn,
        pressed,
    })
}
