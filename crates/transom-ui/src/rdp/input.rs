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

use super::view::{RdpView, RdpViewState};

impl RdpView {
    /// 处理鼠标指针在视口内的移动事件（集成 125Hz 流量整形）。
    pub(crate) fn handle_mouse_move(&mut self, event: &gpui_kit::MouseMoveEvent) {
        if let RdpViewState::Connected { width, height } = self.state
            && let Some((x, y)) = map_pointer_position(
                event.position,
                self.viewport_bounds,
                width,
                height,
                self.is_scaled,
            )
        {
            let is_pos_changed = self.last_mouse_pos != Some((x, y));
            let is_interval_passed =
                self.last_mouse_sent_at.elapsed() >= std::time::Duration::from_millis(8);
            if is_pos_changed && is_interval_passed {
                self.last_mouse_pos = Some((x, y));
                self.last_mouse_sent_at = std::time::Instant::now();
                self.send_input(RdpInput::MouseMove { x, y });
            } else if is_pos_changed {
                self.last_mouse_pos = Some((x, y));
            }
        }
    }

    /// 鼠标移出视口：清理状态并释放可能按住的修饰键。
    pub(crate) fn handle_mouse_exit(&mut self) {
        self.last_mouse_pos = None;
        self.send_input(RdpInput::ReleaseAllKeys);
    }

    /// 处理鼠标按下：同步系统剪贴板、原子补齐最新坐标并派发点击。
    pub(crate) fn handle_mouse_down(
        &mut self,
        btn: gpui_kit::MouseButton,
        cx: &mut gpui_kit::Context<Self>,
    ) {
        self.sync_clipboard_to_remote(cx);
        if let Some((x, y)) = self.last_mouse_pos {
            self.send_input(RdpInput::MouseMove { x, y });
        }
        if let Some(input) = map_mouse_button(btn, true) {
            self.send_input(input);
        }
    }

    /// 处理鼠标抬起。
    pub(crate) fn handle_mouse_up(&mut self, btn: gpui_kit::MouseButton) {
        if let Some(input) = map_mouse_button(btn, false) {
            self.send_input(input);
        }
    }

    /// 处理滚轮事件（行滚动与触控板高精度微像素累加）。
    pub(crate) fn handle_scroll(&mut self, event: &gpui_kit::ScrollEvent) {
        match event.delta {
            gpui_kit::ScrollDelta::Lines(p) => {
                let delta_y = (p.y * 120.0) as i16;
                let delta_x = (p.x * 120.0) as i16;
                if delta_y != 0 {
                    self.send_input(RdpInput::MouseWheel {
                        vertical: true,
                        delta: delta_y,
                    });
                }
                if delta_x != 0 {
                    self.send_input(RdpInput::MouseWheel {
                        vertical: false,
                        delta: delta_x,
                    });
                }
            }
            gpui_kit::ScrollDelta::Pixels(p) => {
                self.scroll_remainder_y += f32::from(p.y);
                self.scroll_remainder_x += f32::from(p.x);

                let steps_y = (self.scroll_remainder_y / 10.0).trunc();
                if steps_y != 0.0 {
                    self.scroll_remainder_y -= steps_y * 10.0;
                    let delta_y = (steps_y * 120.0) as i16;
                    self.send_input(RdpInput::MouseWheel {
                        vertical: true,
                        delta: delta_y,
                    });
                }

                let steps_x = (self.scroll_remainder_x / 10.0).trunc();
                if steps_x != 0.0 {
                    self.scroll_remainder_x -= steps_x * 10.0;
                    let delta_x = (steps_x * 120.0) as i16;
                    self.send_input(RdpInput::MouseWheel {
                        vertical: false,
                        delta: delta_x,
                    });
                }
            }
        }
    }

    /// 处理键盘按键按下（粘贴快捷键感知与扫描码派发）。
    pub(crate) fn handle_key_down(
        &mut self,
        event: &gpui_kit::KeyDownEvent,
        cx: &mut gpui_kit::Context<Self>,
    ) {
        let key = &event.keystroke.key;
        let is_paste = key.eq_ignore_ascii_case("v")
            && (event.keystroke.modifiers.control || event.keystroke.modifiers.platform);
        if is_paste {
            self.sync_clipboard_to_remote(cx);
        }
        self.send_input(RdpInput::KeyDown {
            key: key.clone(),
        });
    }

    /// 处理键盘按键抬起。
    pub(crate) fn handle_key_up(&mut self, event: &gpui_kit::KeyUpEvent) {
        self.send_input(RdpInput::KeyUp {
            key: event.keystroke.key.clone(),
        });
    }
}
