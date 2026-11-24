//! RDP 界面交互输入事件转换与流量整形。
//!
//! 设计原理（Why）：
//! 1. 复用 [`ViewportTransform`] 唯一真源完成鼠标坐标反算，保证渲染与点击像素零偏差；
//! 2. 鼠标移出视口仅清理本地悬停状态（不再误释放正在输入的键盘按键），键盘全释放由
//!    视图失焦钩子统一保障；
//! 3. 鼠标移动采用 125Hz（8ms）流量整形，点击前补发最新坐标，防止高频鼠标事件灌满通道。

use super::frame_cache::ViewportTransform;
use super::view::{RdpView, RdpViewState};
use gpui_kit::{Bounds, Pixels, Point, Window};
use std::time::{Duration, Instant};
use transom_rdp::{ModifierState, MouseBtn, RdpInput};

/// 鼠标移动流量整形间隔（125Hz）。
const MOUSE_MOVE_THROTTLE_INTERVAL: Duration = Duration::from_millis(8);

/// 触控板微像素滚动累加步长（像素）。
const SCROLL_PIXEL_STEP: f32 = 10.0;

/// RDP 滚轮标准单位增量（对应 Windows WHEEL_DELTA）。
const RDP_WHEEL_DELTA: f32 = 120.0;

/// 计算鼠标在远端桌面上的物理像素坐标。
pub fn map_pointer_position(
    cursor_pos: Point<Pixels>,
    viewport_bounds: Bounds<Pixels>,
    desktop_width: u16,
    desktop_height: u16,
    scale_to_fit: bool,
    pan_offset: (f32, f32),
) -> Option<(u16, u16)> {
    let transform = ViewportTransform::compute(
        viewport_bounds,
        desktop_width,
        desktop_height,
        scale_to_fit,
        pan_offset,
    )?;
    transform.unmap_point(viewport_bounds, cursor_pos, desktop_width, desktop_height)
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

/// 将 GPUI 修饰键状态转换为协议层 [`ModifierState`]。
pub fn map_modifiers(modifiers: gpui_kit::Modifiers) -> ModifierState {
    ModifierState {
        ctrl: modifiers.control,
        alt: modifiers.alt,
        shift: modifiers.shift,
        win: modifiers.platform,
    }
}

impl RdpView {
    /// 处理鼠标指针在视口内的移动事件（集成 125Hz 流量整形与坐标无重入缓存）。
    pub(crate) fn handle_mouse_move(
        &mut self,
        event: &gpui_kit::MouseMoveEvent,
        cx: &mut gpui_kit::Context<Self>,
    ) {
        let RdpViewState::Connected { width, height } = self.state else {
            return;
        };
        let bounds = self.viewport_bounds.get();
        let mapped = map_pointer_position(
            event.position,
            bounds,
            width,
            height,
            self.is_scaled,
            self.input.pan_offset,
        );

        if mapped != self.input.last_mouse_pos.get() {
            self.input.last_mouse_pos.set(mapped);
            if self.is_custom_cursor_active() {
                cx.notify();
            }
        }

        if let Some((x, y)) = mapped
            && self.input.last_mouse_sent_at.elapsed() >= MOUSE_MOVE_THROTTLE_INTERVAL
        {
            self.input.last_mouse_sent_at = Instant::now();
            self.send_input(RdpInput::MouseMove { x, y });
        }
    }

    /// 鼠标移出视口：仅清理悬停坐标状态，不干扰仍在焦点的键盘输入。
    pub(crate) fn handle_mouse_exit(&mut self, cx: &mut gpui_kit::Context<Self>) {
        if self.input.last_mouse_pos.replace(None).is_some() && self.is_custom_cursor_active() {
            cx.notify();
        }
    }

    /// 处理鼠标按下：夺回键盘焦点、同步系统剪贴板、原子补齐最新坐标并派发点击。
    pub(crate) fn handle_mouse_down(
        &mut self,
        btn: gpui_kit::MouseButton,
        window: &mut Window,
        cx: &mut gpui_kit::Context<Self>,
    ) {
        self.focus(window, cx);
        self.sync_clipboard_to_remote(cx);
        if let Some((x, y)) = self.input.last_mouse_pos.get() {
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

    /// 处理滚轮事件：1:1 原尺寸模式下按住 Shift 可平移超出视口的桌面区域，否则透传远端滚轮。
    pub(crate) fn handle_scroll(
        &mut self,
        event: &gpui_kit::ScrollEvent,
        cx: &mut gpui_kit::Context<Self>,
    ) {
        if !self.is_scaled && event.modifiers.shift {
            self.pan_viewport(event.delta, cx);
            return;
        }
        match event.delta {
            gpui_kit::ScrollDelta::Lines(p) => {
                self.dispatch_wheel_delta(true, (p.y * RDP_WHEEL_DELTA) as i16);
                self.dispatch_wheel_delta(false, (p.x * RDP_WHEEL_DELTA) as i16);
            }
            gpui_kit::ScrollDelta::Pixels(p) => {
                self.input.scroll_remainder_y += f32::from(p.y);
                self.input.scroll_remainder_x += f32::from(p.x);

                let steps_y = (self.input.scroll_remainder_y / SCROLL_PIXEL_STEP).trunc();
                if steps_y != 0.0 {
                    self.input.scroll_remainder_y -= steps_y * SCROLL_PIXEL_STEP;
                    self.dispatch_wheel_delta(true, (steps_y * RDP_WHEEL_DELTA) as i16);
                }

                let steps_x = (self.input.scroll_remainder_x / SCROLL_PIXEL_STEP).trunc();
                if steps_x != 0.0 {
                    self.input.scroll_remainder_x -= steps_x * SCROLL_PIXEL_STEP;
                    self.dispatch_wheel_delta(false, (steps_x * RDP_WHEEL_DELTA) as i16);
                }
            }
        }
    }

    /// 在 1:1 原始比例模式下平移本地视口偏移，便于查看超出窗格范围的远端区域。
    fn pan_viewport(&mut self, delta: gpui_kit::ScrollDelta, cx: &mut gpui_kit::Context<Self>) {
        let RdpViewState::Connected { width, height } = self.state else {
            return;
        };
        let (dx, dy) = match delta {
            gpui_kit::ScrollDelta::Lines(p) => (p.x * 48.0, p.y * 48.0),
            gpui_kit::ScrollDelta::Pixels(p) => (f32::from(p.x), f32::from(p.y)),
        };
        let bounds = self.viewport_bounds.get();
        let min_x = (f32::from(bounds.size.width) - f32::from(width)).min(0.0);
        let min_y = (f32::from(bounds.size.height) - f32::from(height)).min(0.0);
        let next = (
            (self.input.pan_offset.0 + dx).clamp(min_x, 0.0),
            (self.input.pan_offset.1 + dy).clamp(min_y, 0.0),
        );
        if next != self.input.pan_offset {
            self.input.pan_offset = next;
            cx.notify();
        }
    }

    /// 派发非零滚轮增量指令。
    fn dispatch_wheel_delta(&self, vertical: bool, delta: i16) {
        if delta != 0 {
            self.send_input(RdpInput::MouseWheel { vertical, delta });
        }
    }

    /// 处理独立修饰键变更（Ctrl / Alt / Shift / Win 按下与抬起）。
    pub(crate) fn handle_modifiers_changed(&mut self, event: &gpui_kit::ModifiersChangedEvent) {
        self.send_input(RdpInput::ModifiersChanged(map_modifiers(event.modifiers)));
    }

    /// 处理键盘按键按下（粘贴快捷键感知、修饰键同步与扫描码/Unicode 派发）。
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
            key_char: event.keystroke.key_char.clone(),
            modifiers: map_modifiers(event.keystroke.modifiers),
        });
    }

    /// 处理键盘按键抬起。
    pub(crate) fn handle_key_up(&mut self, event: &gpui_kit::KeyUpEvent) {
        self.send_input(RdpInput::KeyUp {
            key: event.keystroke.key.clone(),
            key_char: event.keystroke.key_char.clone(),
        });
    }
}
