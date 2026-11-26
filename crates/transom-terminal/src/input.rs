//! GPUI 按键事件到终端核心编码器的适配层。
//!
//! 设计原理（Why）：按键编码属于终端核心，但 `gpui_kit::Keystroke` 是 UI 输入
//! 事件类型。这里只做一次字段映射，核心因此不依赖 GPUI，同时保持 crate 既有
//! 公开函数签名不变。

use alacritty_terminal::term::TermMode;
pub use transom_terminal_core::input::{
    KeyEventKind, MouseButtonCode, encode_mouse, focus_bytes, paste_bytes,
};
use transom_terminal_core::input::{
    TerminalKeystroke, TerminalModifiers, keystroke_event_to_bytes as encode_key_event,
    keystroke_to_bytes as encode_key,
};

/// 把一次 GPUI 按键事件编成终端字节。
pub fn keystroke_event_to_bytes(
    keystroke: &gpui_kit::Keystroke,
    mode: TermMode,
    kind: KeyEventKind,
) -> Option<Vec<u8>> {
    encode_key_event(&terminal_keystroke(keystroke), mode, kind)
}

/// 把一次 GPUI 按键编成传统终端字节序列。
pub fn keystroke_to_bytes(keystroke: &gpui_kit::Keystroke, mode: TermMode) -> Option<Vec<u8>> {
    encode_key(&terminal_keystroke(keystroke), mode)
}

fn terminal_keystroke(keystroke: &gpui_kit::Keystroke) -> TerminalKeystroke {
    TerminalKeystroke {
        key: keystroke.key.clone(),
        key_char: keystroke.key_char.clone(),
        modifiers: TerminalModifiers {
            control: keystroke.modifiers.control,
            alt: keystroke.modifiers.alt,
            shift: keystroke.modifiers.shift,
            platform: keystroke.modifiers.platform,
            function: keystroke.modifiers.function,
        },
    }
}

/// 把 Tab/BackTab 编成当前终端模式要求的字节。
pub fn tab_bytes(mode: TermMode, backtab: bool) -> Vec<u8> {
    transom_terminal_core::input::tab_bytes(mode, backtab)
}
