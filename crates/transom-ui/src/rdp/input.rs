//! RDP 界面交互输入事件转换。
//!
//! 设计原理（Why）：GPUI 界面层传递的坐标是带有窗格偏移、外边距与可能的
//! 缩放（FitWindow）的浮点逻辑像素；需要基于当前视口包围盒进行归一化并
//! 映射到远端桌面的实际物理像素坐标系，同时过滤掉视口外的越界坐标。

use gpui_kit::{Bounds, Pixels, Point};
use transom_rdp::{MouseBtn, RdpInput};

/// 计算鼠标在远端桌面上的物理像素坐标。
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

    let (x, y) = if scale_to_fit {
        let view_w = f32::from(viewport_bounds.size.width);
        let view_h = f32::from(viewport_bounds.size.height);
        let scale_x = f32::from(desktop_width) / view_w;
        let scale_y = f32::from(desktop_height) / view_h;
        (rel_x * scale_x, rel_y * scale_y)
    } else {
        // 非等比缩放模式下，若鼠标落在超出桌面尺寸的黑边空白区域，直接忽略
        if rel_x >= f32::from(desktop_width) || rel_y >= f32::from(desktop_height) {
            return None;
        }
        (rel_x, rel_y)
    };

    if x < 0.0 || y < 0.0 {
        return None;
    }

    let px = (x.round() as u16).min(desktop_width - 1);
    let py = (y.round() as u16).min(desktop_height - 1);
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
