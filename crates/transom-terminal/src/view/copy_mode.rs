//! 键盘复制模式（vi 风格）：按键状态机与选区推进。
//!
//! 设计原理（Why）：
//! - 实现初衷：终端老用户习惯不碰鼠标就把上次输出抠出来。原先只有鼠标
//!   选择，键盘用户在多屏滚动的长命令输出前只能反复拖拽。
//! - 核心优势：光标移动直接复用 `alacritty_terminal` 的 `ViModeCursor` 与
//!   `vi_motion`（折行、宽字符、词边界都已在核心里实现），选区复用它的
//!   `Selection`，本模块只负责"哪根键对应哪个动作"与视觉反馈，不重造引擎。
//! - 代价与局限：没有实现 `/` 反向检索与行号跳转；动作集合覆盖日常的移动、
//!   按词跳转、行选与复制，超出范围的键在模式内被吞掉而不是透传给远端。

use super::TerminalView;
use crate::event::TerminalEvent;
use alacritty_terminal::grid::Scroll;
use alacritty_terminal::selection::SelectionType;
use alacritty_terminal::vi_mode::ViMotion;
use gpui_kit::*;

impl TerminalView {
    /// 当前是否处于键盘复制模式。
    pub(crate) fn copy_mode_active(&self) -> bool {
        self.state.engine.lock().copy_mode_active()
    }

    /// 进入/退出键盘复制模式，返回切换后的状态。
    ///
    /// 退出时连同选区一起清空：把光标交回 Shell 后还留着高亮，会被误认为
    /// "仍然选中"，再按 Ctrl+Shift+C 复制到的是上一次的内容。
    pub fn toggle_copy_mode(&mut self, cx: &mut Context<Self>) -> bool {
        let active = {
            let mut engine = self.state.engine.lock();
            let active = engine.toggle_copy_mode();
            if !active {
                engine.clear_selection();
            }
            active
        };
        self.copy_anchor = None;
        cx.emit(TerminalEvent::CopyModeChanged { active });
        cx.notify();
        active
    }

    /// 处理复制模式下的按键；所有按键都在模式内消费，不写入远端。
    pub(super) fn handle_copy_mode_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        cx.stop_propagation();
        let key = event.keystroke.key.to_lowercase();
        let modifiers = event.keystroke.modifiers;
        let control = modifiers.control || modifiers.platform;
        let shift = modifiers.shift;
        // 可打印字符以 key_char 为准：Shift+4 的 key 是 "4"、key_char 是 "$"，
        // 只按 key 匹配会把所有带 Shift 的符号键都错认成数字键。
        let ch = event.keystroke.key_char.clone();
        let ch = ch.as_deref().unwrap_or_default();

        // 复制/粘贴组合键保持可用：习惯 Ctrl+Shift+C 的用户不必先记 y。
        if control && key == "c" {
            self.copy_selection(cx);
            self.exit_copy_mode(cx);
            return;
        }
        if control && key == "v" {
            self.exit_copy_mode(cx);
            self.paste_from_clipboard(cx);
            return;
        }

        match key.as_str() {
            "escape" => {
                self.exit_copy_mode(cx);
                return;
            }
            "q" if !control => {
                self.exit_copy_mode(cx);
                return;
            }
            "y" if !control => {
                self.copy_selection(cx);
                self.exit_copy_mode(cx);
                return;
            }
            "enter" => {
                self.copy_selection(cx);
                self.exit_copy_mode(cx);
                return;
            }
            "v" if !control => {
                // V（Shift+V）按整行选，v 按字符选；重复按取消选择。
                let ty = if shift {
                    SelectionType::Lines
                } else {
                    SelectionType::Simple
                };
                self.toggle_copy_selection(ty, cx);
                return;
            }
            "h" | "left" => self.copy_motion(ViMotion::Left, cx),
            "j" | "down" => self.copy_motion(ViMotion::Down, cx),
            "k" | "up" => self.copy_motion(ViMotion::Up, cx),
            "l" | "right" => self.copy_motion(ViMotion::Right, cx),
            "w" if !control => self.copy_motion(ViMotion::WordRight, cx),
            "b" if !control => self.copy_motion(ViMotion::WordLeft, cx),
            "e" if !control => self.copy_motion(ViMotion::WordRightEnd, cx),
            // Shift+0 是 ")"（key_char），不能落进"行首"分支。
            "0" if !control && !shift => self.copy_motion(ViMotion::First, cx),
            "home" => self.copy_motion(ViMotion::First, cx),
            "end" => self.copy_motion(ViMotion::Last, cx),
            "g" if !control => {
                // vi 的 gg/G：屏幕首行与末行；不区分按键大小写时用 Shift 判定。
                let motion = if shift { ViMotion::Low } else { ViMotion::High };
                self.copy_motion(motion, cx);
            }
            "pageup" => self.copy_motion(ViMotion::High, cx),
            "pagedown" => self.copy_motion(ViMotion::Low, cx),
            _ => match ch {
                "$" => self.copy_motion(ViMotion::Last, cx),
                "^" => self.copy_motion(ViMotion::FirstOccupied, cx),
                "{" => self.copy_motion(ViMotion::ParagraphUp, cx),
                "}" => self.copy_motion(ViMotion::ParagraphDown, cx),
                _ => {}
            },
        }
        cx.notify();
    }

    /// 移动复制光标；若已进入选择，同步扩展选区。
    fn copy_motion(&mut self, motion: ViMotion, cx: &mut Context<Self>) {
        let mut engine = self.state.engine.lock();
        engine.vi_motion(motion);
        if self.copy_anchor.is_some() {
            let point = engine.vi_cursor_point();
            engine.extend_keyboard_selection(point);
        }
        drop(engine);
        cx.notify();
    }

    /// 开始或取消键盘选区。
    fn toggle_copy_selection(&mut self, ty: SelectionType, cx: &mut Context<Self>) {
        if self.copy_anchor.take().is_some() {
            self.state.engine.lock().clear_selection();
        } else {
            let mut engine = self.state.engine.lock();
            let point = engine.vi_cursor_point();
            engine.begin_keyboard_selection(ty, point);
            // 同点扩展一次：让起点的单元格立刻进入选区，按下 v 就有可见反馈。
            engine.extend_keyboard_selection(point);
            self.copy_anchor = Some(point);
        }
        cx.notify();
    }

    /// 退出复制模式并清空选区（幂等）。
    pub(super) fn exit_copy_mode(&mut self, cx: &mut Context<Self>) {
        {
            let mut engine = self.state.engine.lock();
            if engine.copy_mode_active() {
                engine.toggle_copy_mode();
            }
            engine.clear_selection();
        }
        self.copy_anchor = None;
        // 退出时把视口带回底部：停在历史里会让提示符"消失"，用户会以为
        // Shell 卡死；这也是 tmux 复制模式退出后的行为。
        self.state.engine.lock().scroll_display(Scroll::Bottom);
        cx.emit(TerminalEvent::CopyModeChanged { active: false });
        cx.notify();
    }
}
