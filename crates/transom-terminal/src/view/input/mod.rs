//! 终端键鼠事件处理、选区拖拽与滚动响应。
//!
//! 键盘按键与自定义键映射分别放在 keyboard.rs / custom_keys.rs。

use super::{SendBacktab, SendTab, TerminalCopy, TerminalPaste, TerminalView};
use crate::input::{MouseButtonCode, encode_mouse};
use alacritty_terminal::grid::Scroll;
use alacritty_terminal::index::Side;
use alacritty_terminal::selection::SelectionType;
use alacritty_terminal::term::TermMode;
use gpui_kit::*;
use transom_core::RightClickBehavior;

mod custom_keys;
mod keyboard;

impl TerminalView {
    pub(super) fn send_mouse_event(
        &mut self,
        mode: TermMode,
        button: MouseButtonCode,
        pressed: bool,
        motion: bool,
        position: Point<Pixels>,
    ) {
        let Some((row, col)) = self.hit_test_screen_cell(position) else {
            return;
        };
        if let Some(bytes) = encode_mouse(mode, button, pressed, motion, col, row) {
            self.write_input(&bytes);
        }
    }

    pub(super) fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 刻度带点击优先于文本选择与焦点切换：此时输入框必须保住焦点。
        if self.search_strip_click(event.position, window, cx) {
            return;
        }
        window.focus(&self.focus_handle, cx);
        // 鼠标点击先退出复制模式，再按常规路径处理：用户点向某处时意思就是
        // "回到这里"，继续停在旧光标上会被当成失灵。不做 return，本次点击
        // 仍可正常起一个鼠标选区。
        if self.copy_mode_active() {
            self.exit_copy_mode(cx);
        }
        if let Some(block_id) = self.banner_at_position(event.position) {
            self.toggle_block(block_id, cx);
            return;
        }
        let mode = self.state.engine.lock().mode();

        match event.button {
            MouseButton::Right => {
                // 三档语义：Paste 直接粘贴；Menu / MenuWithPaste 由
                // ContextMenuExt 打开菜单，这里不再消费点击。
                if self.interaction.right_click_behavior() == RightClickBehavior::Paste {
                    self.paste_from_clipboard(cx);
                }
            }
            MouseButton::Left => {
                // Ctrl+点击优先尝试打开 OSC 8 超链接：命中即消费本次点击，
                // 未命中回落到既有的选择/上报行为，不改变其他修饰键语义。
                if event.modifiers.control && self.open_hyperlink_at(event.position, cx) {
                    return;
                }
                // Alt 拖拽 = 矩形列选。它和 Shift 一样压制程序自己的鼠标上报：
                // 需要抠列的场景恰恰发生在 vim/htop 这类接管鼠标的程序里，
                // 若让位给上报，这个功能在最需要它的界面上就是空摆设。
                let rectangular = event.modifiers.alt;
                if mode.intersects(TermMode::MOUSE_MODE) && !event.modifiers.shift && !rectangular {
                    self.send_mouse_event(mode, MouseButtonCode::Left, true, false, event.position);
                    self.mouse_pressed = true;
                    return;
                }
                if let Some(point) = self.hit_test_grid_point(event.position) {
                    // 双击/三击的选词与选行语义只在矩形模式之外生效：Alt 按下时
                    // 用户要的是"这一块"，不是"这个词"。
                    let ty = if rectangular {
                        SelectionType::Block
                    } else {
                        match event.click_count {
                            2 => SelectionType::Semantic,
                            3.. => SelectionType::Lines,
                            _ => SelectionType::Simple,
                        }
                    };
                    self.state
                        .engine
                        .lock()
                        .start_selection(ty, point, Side::Left);
                    self.dragging_selection = true;
                    cx.notify();
                }
            }
            MouseButton::Middle if mode.intersects(TermMode::MOUSE_MODE) => {
                self.send_mouse_event(mode, MouseButtonCode::Middle, true, false, event.position);
            }
            _ => {}
        }
    }

    pub(super) fn on_mouse_up(
        &mut self,
        event: &MouseUpEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.button != MouseButton::Left {
            return;
        }
        let mode = self.state.engine.lock().mode();
        if self.mouse_pressed {
            self.send_mouse_event(mode, MouseButtonCode::Left, false, false, event.position);
            self.mouse_pressed = false;
            return;
        }
        if self.dragging_selection {
            if let Some(point) = self.hit_test_grid_point(event.position) {
                self.state
                    .engine
                    .lock()
                    .update_selection(point, Side::Right);
            }
            self.dragging_selection = false;
            if self.interaction.copy_on_select {
                self.copy_selection(cx);
            }
            cx.notify();
        }
    }

    pub(super) fn on_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mode = self.state.engine.lock().mode();
        if self.mouse_pressed && mode.intersects(TermMode::MOUSE_MODE) {
            let wants_motion = mode.contains(TermMode::MOUSE_MOTION)
                || (mode.contains(TermMode::MOUSE_DRAG) && event.dragging());
            if wants_motion {
                self.send_mouse_event(mode, MouseButtonCode::Left, true, true, event.position);
            }
            return;
        }
        if self.dragging_selection && event.dragging() {
            // 指针在视口外（上方历史区）时 hit_test 落空，选区更新交给
            // 边缘自动滚动后的重映射完成
            if let Some(point) = self.hit_test_grid_point(event.position) {
                self.state
                    .engine
                    .lock()
                    .update_selection(point, Side::Right);
            }
            self.edge_scroll_during_drag(event.position, cx);
            cx.notify();
        }
        self.update_hover_block(event.position, cx);
        // 拖拽选区时不切手型：此刻用户的操作对象是文本，不是链接。
        if !self.mouse_pressed && !self.dragging_selection {
            self.update_link_cursor(event.position, window);
        }
    }

    pub(super) fn edge_scroll_during_drag(
        &mut self,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let metrics = *self.state.metrics.lock();
        let cell_height = f32::from(metrics.cell_height);
        if cell_height <= 0.0 {
            return;
        }
        let y = f32::from(position.y);
        let top = f32::from(metrics.origin.y);
        let edge = cell_height * 1.5;
        // 视口符号约定与滚轮一致：正数向上翻历史，负数向下回底部
        let delta = if y < top + edge {
            1
        } else if y > top + cell_height * metrics.rows as f32 - edge {
            -1
        } else {
            return;
        };
        self.state
            .engine
            .lock()
            .scroll_display(Scroll::Delta(delta));
        if let Some(point) = self.hit_test_grid_point(position) {
            self.state
                .engine
                .lock()
                .update_selection(point, Side::Right);
        }
        cx.notify();
    }

    pub(super) fn on_scroll(
        &mut self,
        event: &ScrollWheelEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let lines = match event.delta {
            ScrollDelta::Lines(delta) => delta.y.round() as i32,
            ScrollDelta::Pixels(delta) => {
                let metrics = *self.state.metrics.lock();
                let height = f32::from(metrics.cell_height);
                if height <= 0.0 {
                    0
                } else {
                    self.scroll_pixel_remainder += f32::from(delta.y) / height;
                    let lines = self.scroll_pixel_remainder.round() as i32;
                    self.scroll_pixel_remainder -= lines as f32;
                    lines
                }
            }
        };
        if lines == 0 {
            return;
        }

        let mode = self.state.engine.lock().mode();
        if mode.intersects(TermMode::MOUSE_MODE) && !event.modifiers.shift {
            let button = if lines > 0 {
                MouseButtonCode::WheelUp
            } else {
                MouseButtonCode::WheelDown
            };
            for _ in 0..lines.abs().min(5) {
                self.send_mouse_event(mode, button, true, false, event.position);
            }
        } else if mode.contains(TermMode::ALT_SCREEN) && self.profile.alternate_scroll {
            // 备屏没有可滚动的历史缓冲（scroll_display 是空操作），滚轮
            // 转换为方向键发给全屏程序，与主流终端的 alternate scroll 语义一致。
            let sequence: &[u8] = if lines > 0 { b"\x1b[A" } else { b"\x1b[B" };
            for _ in 0..lines.abs().min(5) {
                self.write_input(sequence);
            }
        } else {
            self.state
                .engine
                .lock()
                .scroll_display(Scroll::Delta(lines));
            cx.notify();
        }
    }
}

pub(crate) use custom_keys::build_custom_keys;
