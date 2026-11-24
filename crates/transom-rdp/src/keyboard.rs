//! 键盘扫描码映射与输入状态机。
//!
//! 设计原理（Why）：GPUI 界面层传递的是逻辑按键描述符（如字符串键名与修饰键掩码），
//! 而 RDP 协议在线路上要求传输标准的 PC XT/AT 扫描码（Scancode）。
//! 本模块建立标准扫描码转换表，并维护修饰键状态，避免远端按键卡死或错乱。

use ironrdp_input::{Operation, Scancode};
use std::collections::HashSet;

/// 键盘状态跟踪器。
///
/// 维护当前处于按下状态的物理扫描码集合，在视图失焦或会话断开时精准释放，
/// 避免修饰键或普通按键在远端桌面发生粘滞。
#[derive(Default)]
pub struct KeyboardTranslator {
    pressed_keys: HashSet<Scancode>,
}

impl KeyboardTranslator {
    /// 处理按键按下，返回一组需要送往 RDP 输入数据库的底层原子操作。
    pub fn handle_key_down(&mut self, key_name: &str) -> Vec<Operation> {
        let mut ops = Vec::with_capacity(1);
        if let Some(scancode) = key_name_to_scancode(key_name) {
            self.pressed_keys.insert(scancode);
            ops.push(Operation::KeyPressed(scancode));
        }
        ops
    }

    /// 处理按键抬起。
    pub fn handle_key_up(&mut self, key_name: &str) -> Vec<Operation> {
        let mut ops = Vec::with_capacity(1);
        if let Some(scancode) = key_name_to_scancode(key_name) {
            self.pressed_keys.remove(&scancode);
            ops.push(Operation::KeyReleased(scancode));
        }
        ops
    }

    /// 释放全部当前被按下的按键（防止切换标签或失焦时遗留按键按住状态）。
    pub fn release_all(&mut self) -> Vec<Operation> {
        if self.pressed_keys.is_empty() {
            // 兜底释放关键修饰键
            return vec![
                Operation::KeyReleased(Scancode::from_u8(false, 0x1D)), // Left Ctrl
                Operation::KeyReleased(Scancode::from_u8(false, 0x38)), // Left Alt
                Operation::KeyReleased(Scancode::from_u8(false, 0x2A)), // Left Shift
                Operation::KeyReleased(Scancode::from_u8(true, 0x5B)),  // Left Win
            ];
        }

        self.pressed_keys
            .drain()
            .map(Operation::KeyReleased)
            .collect()
    }
}

/// 常用键名到 XT 扫描码（及扩展标志）映射。
///
/// 使用栈上小缓冲完成 ASCII 小写归一化，避免每次键盘事件触发堆分配。
fn key_name_to_scancode(name: &str) -> Option<Scancode> {
    if name.len() > 24 {
        return None;
    }
    let mut buf = [0u8; 24];
    for (dst, &src) in buf.iter_mut().zip(name.as_bytes()) {
        *dst = src.to_ascii_lowercase();
    }
    let lower = std::str::from_utf8(&buf[..name.len()]).ok()?;
    let code = match lower {
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
        "capslock" | "caps" => Scancode::from_u8(false, 0x3A),
        // 功能键 F1 - F12
        "f1" => Scancode::from_u8(false, 0x3B),
        "f2" => Scancode::from_u8(false, 0x3C),
        "f3" => Scancode::from_u8(false, 0x3D),
        "f4" => Scancode::from_u8(false, 0x3E),
        "f5" => Scancode::from_u8(false, 0x3F),
        "f6" => Scancode::from_u8(false, 0x40),
        "f7" => Scancode::from_u8(false, 0x41),
        "f8" => Scancode::from_u8(false, 0x42),
        "f9" => Scancode::from_u8(false, 0x43),
        "f10" => Scancode::from_u8(false, 0x44),
        "f11" => Scancode::from_u8(false, 0x57),
        "f12" => Scancode::from_u8(false, 0x58),
        // 常用标点符号与 Shift 组合符号（PC XT 物理键位映射）
        "[" | "bracketleft" | "{" | "braceleft" => Scancode::from_u8(false, 0x1A),
        "]" | "bracketright" | "}" | "braceright" => Scancode::from_u8(false, 0x1B),
        ";" | "semicolon" | ":" | "colon" => Scancode::from_u8(false, 0x27),
        "'" | "quote" | "\"" | "quotedbl" => Scancode::from_u8(false, 0x28),
        "`" | "grave" | "~" | "asciitilde" => Scancode::from_u8(false, 0x29),
        "\\" | "backslash" | "|" | "bar" => Scancode::from_u8(false, 0x2B),
        "," | "comma" | "<" | "less" => Scancode::from_u8(false, 0x33),
        "." | "period" | ">" | "greater" => Scancode::from_u8(false, 0x34),
        "/" | "slash" | "?" | "question" => Scancode::from_u8(false, 0x35),
        "!" | "exclam" => Scancode::from_u8(false, 0x02),
        "@" | "at" => Scancode::from_u8(false, 0x03),
        "#" | "numbersign" => Scancode::from_u8(false, 0x04),
        "$" | "dollar" => Scancode::from_u8(false, 0x05),
        "%" | "percent" => Scancode::from_u8(false, 0x06),
        "^" | "asciicircum" => Scancode::from_u8(false, 0x07),
        "&" | "ampersand" => Scancode::from_u8(false, 0x08),
        "*" | "asterisk" => Scancode::from_u8(false, 0x09),
        "(" | "parenleft" => Scancode::from_u8(false, 0x0A),
        ")" | "parenright" => Scancode::from_u8(false, 0x0B),
        "_" | "underscore" => Scancode::from_u8(false, 0x0C),
        "+" | "plus" => Scancode::from_u8(false, 0x0D),
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
        // 打印屏幕、滚动锁定与数字锁定
        "printscreen" | "prtscn" => Scancode::from_u8(true, 0x37),
        "scrolllock" => Scancode::from_u8(false, 0x46),
        "numlock" => Scancode::from_u8(false, 0x45),
        // 小键盘（Numpad）按键
        "numpad0" => Scancode::from_u8(false, 0x52),
        "numpad1" => Scancode::from_u8(false, 0x4F),
        "numpad2" => Scancode::from_u8(false, 0x50),
        "numpad3" => Scancode::from_u8(false, 0x51),
        "numpad4" => Scancode::from_u8(false, 0x4B),
        "numpad5" => Scancode::from_u8(false, 0x4C),
        "numpad6" => Scancode::from_u8(false, 0x4D),
        "numpad7" => Scancode::from_u8(false, 0x47),
        "numpad8" => Scancode::from_u8(false, 0x48),
        "numpad9" => Scancode::from_u8(false, 0x49),
        "numpadadd" => Scancode::from_u8(false, 0x4E),
        "numpadsubtract" => Scancode::from_u8(false, 0x4A),
        "numpadmultiply" => Scancode::from_u8(false, 0x37),
        "numpaddivide" => Scancode::from_u8(true, 0x35),
        "numpadenter" => Scancode::from_u8(true, 0x1C),
        "numpaddecimal" => Scancode::from_u8(false, 0x53),
        // 区分右侧功能与修饰键
        "rightctrl" | "rctrl" => Scancode::from_u8(true, 0x1D),
        "rightalt" | "ralt" => Scancode::from_u8(true, 0x38),
        "rightshift" | "rshift" => Scancode::from_u8(false, 0x36),
        "contextmenu" | "menu" | "app" => Scancode::from_u8(true, 0x5D),
        _ => return None,
    };
    Some(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_keyboard_translator_flow() {
        let mut translator = KeyboardTranslator::default();
        let ops = translator.handle_key_down("enter");
        assert_eq!(ops.len(), 1);

        // 按住状态下执行 release_all，应精准释放该按键
        let released = translator.release_all();
        assert_eq!(released.len(), 1);

        // 集合已清空，兜底释放 4 个常见修饰键
        let empty_released = translator.release_all();
        assert_eq!(empty_released.len(), 4);
    }

    #[test]
    fn test_function_keys_and_symbols() {
        assert!(key_name_to_scancode("f5").is_some());
        assert!(key_name_to_scancode("f12").is_some());
        assert!(key_name_to_scancode("backslash").is_some());
        assert!(key_name_to_scancode("numpad0").is_some());
        assert!(key_name_to_scancode("prtscn").is_some());
        assert!(key_name_to_scancode("rctrl").is_some());
        assert!(key_name_to_scancode("!").is_some());
        assert!(key_name_to_scancode("@").is_some());
        assert!(key_name_to_scancode("#").is_some());
        assert!(key_name_to_scancode("$").is_some());
        assert!(key_name_to_scancode(":").is_some());
        assert!(key_name_to_scancode("\"").is_some());
        assert!(key_name_to_scancode("?").is_some());
        assert!(key_name_to_scancode("unknown_key_xyz").is_none());
    }
}
