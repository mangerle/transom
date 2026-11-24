//! 键盘扫描码映射、修饰键差分同步与输入状态机。
//!
//! 设计原理（Why）：
//! 1. GPUI 中单独按下/松开修饰键走 `ModifiersChangedEvent`，而非 `KeyDownEvent`；
//!    因此必须维护 [`ModifierState`] 并与远端做差分同步，否则 `Ctrl+C`、`Alt+Tab`、
//!    `Shift+符号` 等组合键在远端全部失效；
//! 2. 优先映射为 PC XT/AT 扫描码（兼容所有快捷键与游戏/终端）；对未映射的非 ASCII
//!    字符（如中文 IME、特殊符号）在未按控制修饰键时自动降级为 RDP FastPath Unicode
//!    输入事件，避免字符静默丢失；
//! 3. 组合键（如 `Ctrl+Alt+Del`）作为原子指令 [`KeyChord`] 一次性展开，避免多次入队
//!    在通道背压时发生半截丢包导致远端卡键。

use ironrdp_input::{Operation, Scancode};
use std::collections::HashSet;

const SCANCODE_LCTRL: Scancode = Scancode::from_u8(false, 0x1D);
const SCANCODE_LALT: Scancode = Scancode::from_u8(false, 0x38);
const SCANCODE_LSHIFT: Scancode = Scancode::from_u8(false, 0x2A);
const SCANCODE_LWIN: Scancode = Scancode::from_u8(true, 0x5B);
const SCANCODE_DELETE: Scancode = Scancode::from_u8(true, 0x53);

/// 当前本地修饰键状态快照（零堆分配，跨通道按值传递）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ModifierState {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
}

impl ModifierState {
    /// 是否按下了会改变字符语义的控制修饰键（Ctrl / Alt / Win）。
    fn has_command_modifier(self) -> bool {
        self.ctrl || self.alt || self.win
    }
}

/// 原子快捷组合键（单条指令入队，杜绝通道满时只发按下不发抬起）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyChord {
    /// 发送 `Ctrl+Alt+Delete`。
    CtrlAltDelete,
    /// 发送左 `Win` 键。
    Win,
}

/// 键盘状态跟踪器。
///
/// 维护当前处于按下状态的物理扫描码、Unicode 字符与修饰键状态，在视图失焦或
/// 会话断开时精准释放，避免修饰键或普通按键在远端桌面发生粘滞。
#[derive(Default)]
pub struct KeyboardTranslator {
    pressed_keys: HashSet<Scancode>,
    pressed_unicode: HashSet<char>,
    modifiers: ModifierState,
}

impl KeyboardTranslator {
    /// 按最新修饰键快照与当前已同步状态做差分，产出必要的修饰键按下/抬起操作。
    pub fn sync_modifiers(&mut self, next: ModifierState, out: &mut Vec<Operation>) {
        self.sync_one_modifier(self.modifiers.ctrl, next.ctrl, SCANCODE_LCTRL, out);
        self.sync_one_modifier(self.modifiers.alt, next.alt, SCANCODE_LALT, out);
        self.sync_one_modifier(self.modifiers.shift, next.shift, SCANCODE_LSHIFT, out);
        self.sync_one_modifier(self.modifiers.win, next.win, SCANCODE_LWIN, out);
        self.modifiers = next;
    }

    /// 处理按键按下：先对齐修饰键状态，再优先映射扫描码，未命中时按需回退到 Unicode 输入。
    pub fn handle_key_down(
        &mut self,
        key_name: &str,
        key_char: Option<&str>,
        modifiers: ModifierState,
        out: &mut Vec<Operation>,
    ) {
        self.sync_modifiers(modifiers, out);

        if let Some(scancode) = key_name_to_scancode(key_name) {
            // 核心左修饰键已由 `sync_modifiers` 差分同步，避免按下修饰键自身时重复入队
            if is_synced_modifier(scancode) {
                return;
            }
            self.pressed_keys.insert(scancode);
            out.push(Operation::KeyPressed(scancode));
            return;
        }

        // 未命中物理扫描码且无控制修饰键时，回退为 FastPath Unicode 字符事件（支持中文/IME）
        if !modifiers.has_command_modifier()
            && let Some(text) = key_char
        {
            for ch in text.chars() {
                if !ch.is_control() {
                    self.pressed_unicode.insert(ch);
                    out.push(Operation::UnicodeKeyPressed(ch));
                }
            }
        }
    }

    /// 处理按键抬起：按扫描码或 Unicode 字符精准释放。
    pub fn handle_key_up(
        &mut self,
        key_name: &str,
        key_char: Option<&str>,
        out: &mut Vec<Operation>,
    ) {
        if let Some(scancode) = key_name_to_scancode(key_name) {
            self.pressed_keys.remove(&scancode);
            out.push(Operation::KeyReleased(scancode));
            return;
        }

        if let Some(text) = key_char {
            for ch in text.chars() {
                if self.pressed_unicode.remove(&ch) {
                    out.push(Operation::UnicodeKeyReleased(ch));
                }
            }
        }
    }

    /// 原子展开预设快捷组合键（按下后逆序释放）。
    pub fn handle_key_chord(&mut self, chord: KeyChord, out: &mut Vec<Operation>) {
        let keys: &[Scancode] = match chord {
            KeyChord::CtrlAltDelete => &[SCANCODE_LCTRL, SCANCODE_LALT, SCANCODE_DELETE],
            KeyChord::Win => &[SCANCODE_LWIN],
        };
        for &code in keys {
            out.push(Operation::KeyPressed(code));
        }
        for &code in keys.iter().rev() {
            out.push(Operation::KeyReleased(code));
        }
    }

    /// 释放全部当前被按下的物理键、Unicode 键与修饰键（视图失焦或断开时调用）。
    pub fn release_all(&mut self, out: &mut Vec<Operation>) {
        self.modifiers = ModifierState::default();

        for ch in self.pressed_unicode.drain() {
            out.push(Operation::UnicodeKeyReleased(ch));
        }

        let mut released_modifiers = [false; 4];
        for code in self.pressed_keys.drain() {
            if code == SCANCODE_LCTRL {
                released_modifiers[0] = true;
            } else if code == SCANCODE_LALT {
                released_modifiers[1] = true;
            } else if code == SCANCODE_LSHIFT {
                released_modifiers[2] = true;
            } else if code == SCANCODE_LWIN {
                released_modifiers[3] = true;
            }
            out.push(Operation::KeyReleased(code));
        }

        // 兜底释放未在 `pressed_keys` 中的核心修饰键，防止宿主系统吞掉抬起事件导致卡键
        let core_modifiers = [
            SCANCODE_LCTRL,
            SCANCODE_LALT,
            SCANCODE_LSHIFT,
            SCANCODE_LWIN,
        ];
        for (released, code) in released_modifiers.into_iter().zip(core_modifiers) {
            if !released {
                out.push(Operation::KeyReleased(code));
            }
        }
    }

    fn sync_one_modifier(
        &mut self,
        current: bool,
        next: bool,
        code: Scancode,
        out: &mut Vec<Operation>,
    ) {
        if current == next {
            return;
        }
        if next {
            self.pressed_keys.insert(code);
            out.push(Operation::KeyPressed(code));
        } else {
            self.pressed_keys.remove(&code);
            out.push(Operation::KeyReleased(code));
        }
    }
}

fn is_synced_modifier(code: Scancode) -> bool {
    code == SCANCODE_LCTRL
        || code == SCANCODE_LALT
        || code == SCANCODE_LSHIFT
        || code == SCANCODE_LWIN
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
        "control" | "ctrl" => SCANCODE_LCTRL,
        "a" => Scancode::from_u8(false, 0x1E),
        "s" => Scancode::from_u8(false, 0x1F),
        "d" => Scancode::from_u8(false, 0x20),
        "f" => Scancode::from_u8(false, 0x21),
        "g" => Scancode::from_u8(false, 0x22),
        "h" => Scancode::from_u8(false, 0x23),
        "j" => Scancode::from_u8(false, 0x24),
        "k" => Scancode::from_u8(false, 0x25),
        "l" => Scancode::from_u8(false, 0x26),
        "shift" => SCANCODE_LSHIFT,
        "z" => Scancode::from_u8(false, 0x2C),
        "x" => Scancode::from_u8(false, 0x2D),
        "c" => Scancode::from_u8(false, 0x2E),
        "v" => Scancode::from_u8(false, 0x2F),
        "b" => Scancode::from_u8(false, 0x30),
        "n" => Scancode::from_u8(false, 0x31),
        "m" => Scancode::from_u8(false, 0x32),
        "alt" | "option" => SCANCODE_LALT,
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
        "delete" => SCANCODE_DELETE,
        "home" => Scancode::from_u8(true, 0x47),
        "end" => Scancode::from_u8(true, 0x4F),
        "pageup" => Scancode::from_u8(true, 0x49),
        "pagedown" => Scancode::from_u8(true, 0x51),
        "super" | "command" | "win" => SCANCODE_LWIN,
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
        let mut ops = Vec::new();
        translator.handle_key_down("enter", None, ModifierState::default(), &mut ops);
        assert_eq!(ops.len(), 1);

        // 按住状态下执行 release_all，应释放该按键并兜底释放核心修饰键
        ops.clear();
        translator.release_all(&mut ops);
        assert_eq!(ops.len(), 5);

        // 集合已清空，兜底释放 4 个常见修饰键
        ops.clear();
        translator.release_all(&mut ops);
        assert_eq!(ops.len(), 4);
    }

    #[test]
    fn test_modifier_diff_and_unicode_fallback() {
        let mut translator = KeyboardTranslator::default();
        let mut ops = Vec::new();

        // 按下 Ctrl+C：应先发出 Left Ctrl 按下，再发出 C 按下
        let ctrl = ModifierState {
            ctrl: true,
            ..Default::default()
        };
        translator.handle_key_down("c", None, ctrl, &mut ops);
        assert_eq!(ops.len(), 2);
        assert!(matches!(ops[0], Operation::KeyPressed(c) if c == SCANCODE_LCTRL));
        assert!(matches!(
            ops[1],
            Operation::KeyPressed(c) if c == Scancode::from_u8(false, 0x2E)
        ));

        // 释放 Ctrl
        ops.clear();
        translator.sync_modifiers(ModifierState::default(), &mut ops);
        assert_eq!(ops.len(), 1);
        assert!(matches!(ops[0], Operation::KeyReleased(c) if c == SCANCODE_LCTRL));

        // 非 ASCII 中文字符回退到 FastPath Unicode 事件
        ops.clear();
        translator.handle_key_down("中", Some("中"), ModifierState::default(), &mut ops);
        assert_eq!(ops.len(), 1);
        assert!(matches!(ops[0], Operation::UnicodeKeyPressed('中')));

        ops.clear();
        translator.handle_key_up("中", Some("中"), &mut ops);
        assert_eq!(ops.len(), 1);
        assert!(matches!(ops[0], Operation::UnicodeKeyReleased('中')));
    }

    #[test]
    fn test_atomic_key_chord() {
        let mut translator = KeyboardTranslator::default();
        let mut ops = Vec::new();
        translator.handle_key_chord(KeyChord::CtrlAltDelete, &mut ops);
        assert_eq!(ops.len(), 6);
        assert!(matches!(ops[0], Operation::KeyPressed(c) if c == SCANCODE_LCTRL));
        assert!(matches!(ops[5], Operation::KeyReleased(c) if c == SCANCODE_LCTRL));
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
        assert!(key_name_to_scancode("?").is_some());
        assert!(key_name_to_scancode("unknown_key_xyz").is_none());
    }
}
