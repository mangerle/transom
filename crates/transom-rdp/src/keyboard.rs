//! 键盘扫描码映射与输入状态机。
//!
//! 设计原理（Why）：GPUI 界面层传递的是逻辑按键描述符（如字符串键名与修饰键掩码），
//! 而 RDP 协议在线路上要求传输标准的 PC XT/AT 扫描码（Scancode）。
//! 本模块建立标准扫描码转换表，并维护修饰键状态，避免远端按键卡死或错乱。

use ironrdp_input::{Operation, Scancode};

/// 键盘状态跟踪器。
#[derive(Default)]
pub struct KeyboardTranslator {
    ctrl_pressed: bool,
    alt_pressed: bool,
    shift_pressed: bool,
    meta_pressed: bool,
}

impl KeyboardTranslator {
    /// 处理按键按下，返回一组需要送往 RDP 输入数据库的底层原子操作。
    pub fn handle_key_down(&mut self, key_name: &str) -> Vec<Operation> {
        let mut ops = Vec::with_capacity(2);
        if let Some(scancode) = key_name_to_scancode(key_name) {
            self.track_modifiers(key_name, true);
            ops.push(Operation::KeyPressed(scancode));
        }
        ops
    }

    /// 处理按键抬起。
    pub fn handle_key_up(&mut self, key_name: &str) -> Vec<Operation> {
        let mut ops = Vec::with_capacity(2);
        if let Some(scancode) = key_name_to_scancode(key_name) {
            self.track_modifiers(key_name, false);
            ops.push(Operation::KeyReleased(scancode));
        }
        ops
    }

    /// 释放全部按键（防止切换标签或失焦时遗留按键按住状态）。
    pub fn release_all(&mut self) -> Vec<Operation> {
        self.ctrl_pressed = false;
        self.alt_pressed = false;
        self.shift_pressed = false;
        self.meta_pressed = false;
        // 逐一释放关键修饰键
        vec![
            Operation::KeyReleased(Scancode::from_u8(false, 0x1D)), // Left Ctrl
            Operation::KeyReleased(Scancode::from_u8(false, 0x38)), // Left Alt
            Operation::KeyReleased(Scancode::from_u8(false, 0x2A)), // Left Shift
            Operation::KeyReleased(Scancode::from_u8(true, 0x5B)),  // Left Win
        ]
    }

    fn track_modifiers(&mut self, key_name: &str, pressed: bool) {
        match key_name {
            "control" | "ctrl" => self.ctrl_pressed = pressed,
            "alt" | "option" => self.alt_pressed = pressed,
            "shift" => self.shift_pressed = pressed,
            "super" | "command" | "win" => self.meta_pressed = pressed,
            _ => {}
        }
    }
}

/// 常用键名到 XT 扫描码（及扩展标志）映射。
fn key_name_to_scancode(name: &str) -> Option<Scancode> {
    let lower = name.to_ascii_lowercase();
    let code = match lower.as_str() {
        "escape" | "esc" => Scancode::from_u8(false, 0x01),
        "1" => Scancode::from_u8(false, 0x02),
        "2" => Scancode::from_u8(false, 0x03),
        "3" => Scancode::from_u8(false, 0x04),
        "4" => Scancode::from_u8(false, 0x05),
        "5" => Scancode::from_u8(false, 0x06),
        "6" => Scancode::from_u8(false, 0x07),
        "7" => Scancode::from_u8(false, 0x08),
        "8" => Scancode::from_u8(false, 0x09),
        "9" => Scancode::from_u8(false, 0x0A),
        "0" => Scancode::from_u8(false, 0x0B),
        "minus" | "-" => Scancode::from_u8(false, 0x0C),
        "equal" | "=" => Scancode::from_u8(false, 0x0D),
        "backspace" => Scancode::from_u8(false, 0x0E),
        "tab" => Scancode::from_u8(false, 0x0F),
        "q" => Scancode::from_u8(false, 0x10),
        "w" => Scancode::from_u8(false, 0x11),
        "e" => Scancode::from_u8(false, 0x12),
        "r" => Scancode::from_u8(false, 0x13),
        "t" => Scancode::from_u8(false, 0x14),
        "y" => Scancode::from_u8(false, 0x15),
        "u" => Scancode::from_u8(false, 0x16),
        "i" => Scancode::from_u8(false, 0x17),
        "o" => Scancode::from_u8(false, 0x18),
        "p" => Scancode::from_u8(false, 0x19),
        "enter" | "return" => Scancode::from_u8(false, 0x1C),
        "control" | "ctrl" => Scancode::from_u8(false, 0x1D),
        "a" => Scancode::from_u8(false, 0x1E),
        "s" => Scancode::from_u8(false, 0x1F),
        "d" => Scancode::from_u8(false, 0x20),
        "f" => Scancode::from_u8(false, 0x21),
        "g" => Scancode::from_u8(false, 0x22),
        "h" => Scancode::from_u8(false, 0x23),
        "j" => Scancode::from_u8(false, 0x24),
        "k" => Scancode::from_u8(false, 0x25),
        "l" => Scancode::from_u8(false, 0x26),
        "shift" => Scancode::from_u8(false, 0x2A),
        "z" => Scancode::from_u8(false, 0x2C),
        "x" => Scancode::from_u8(false, 0x2D),
        "c" => Scancode::from_u8(false, 0x2E),
        "v" => Scancode::from_u8(false, 0x2F),
        "b" => Scancode::from_u8(false, 0x30),
        "n" => Scancode::from_u8(false, 0x31),
        "m" => Scancode::from_u8(false, 0x32),
        "alt" | "option" => Scancode::from_u8(false, 0x38),
        "space" => Scancode::from_u8(false, 0x39),
        // 导航与功能按键（多为扩展扫描码）
        "up" => Scancode::from_u8(true, 0x48),
        "down" => Scancode::from_u8(true, 0x50),
        "left" => Scancode::from_u8(true, 0x4B),
        "right" => Scancode::from_u8(true, 0x4D),
        "insert" => Scancode::from_u8(true, 0x52),
        "delete" => Scancode::from_u8(true, 0x53),
        "home" => Scancode::from_u8(true, 0x47),
        "end" => Scancode::from_u8(true, 0x4F),
        "pageup" => Scancode::from_u8(true, 0x49),
        "pagedown" => Scancode::from_u8(true, 0x51),
        "super" | "command" | "win" => Scancode::from_u8(true, 0x5B),
        _ => return None,
    };
    Some(code)
}
