use alacritty_terminal::term::TermMode;
use gpui_kit::Keystroke;

pub fn keystroke_to_bytes(keystroke: &Keystroke, mode: TermMode) -> Option<Vec<u8>> {
    if keystroke.modifiers.platform {
        return None;
    }

    match keystroke.key.as_str() {
        "space" => {
            return Some(if keystroke.modifiers.control {
                vec![0]
            } else {
                b" ".to_vec()
            });
        }
        "enter" => return Some(b"\r".to_vec()),
        "escape" => return Some(b"\x1b".to_vec()),
        "backspace" => return Some(b"\x7f".to_vec()),
        "tab" => {
            return Some(if keystroke.modifiers.shift {
                b"\x1b[Z".to_vec()
            } else {
                b"\t".to_vec()
            });
        }
        "up" => return Some(arrow(mode, b'A')),
        "down" => return Some(arrow(mode, b'B')),
        "right" => return Some(arrow(mode, b'C')),
        "left" => return Some(arrow(mode, b'D')),
        "home" => {
            return Some(if mode.contains(TermMode::APP_CURSOR) {
                b"\x1bOH".to_vec()
            } else {
                b"\x1b[H".to_vec()
            });
        }
        "end" => {
            return Some(if mode.contains(TermMode::APP_CURSOR) {
                b"\x1bOF".to_vec()
            } else {
                b"\x1b[F".to_vec()
            });
        }
        "pageup" => return Some(b"\x1b[5~".to_vec()),
        "pagedown" => return Some(b"\x1b[6~".to_vec()),
        "insert" => return Some(b"\x1b[2~".to_vec()),
        "delete" => return Some(b"\x1b[3~".to_vec()),
        "f1" => return Some(b"\x1bOP".to_vec()),
        "f2" => return Some(b"\x1bOQ".to_vec()),
        "f3" => return Some(b"\x1bOR".to_vec()),
        "f4" => return Some(b"\x1bOS".to_vec()),
        "f5" => return Some(b"\x1b[15~".to_vec()),
        "f6" => return Some(b"\x1b[17~".to_vec()),
        "f7" => return Some(b"\x1b[18~".to_vec()),
        "f8" => return Some(b"\x1b[19~".to_vec()),
        "f9" => return Some(b"\x1b[20~".to_vec()),
        "f10" => return Some(b"\x1b[21~".to_vec()),
        "f11" => return Some(b"\x1b[23~".to_vec()),
        "f12" => return Some(b"\x1b[24~".to_vec()),
        _ => {}
    }

    if keystroke.modifiers.control
        && let Some(ch) = single_char(&keystroke.key)
    {
        if ch.is_ascii_alphabetic() {
            return Some(vec![ch.to_ascii_uppercase() as u8 - b'@']);
        }
        match ch {
            '[' => return Some(b"\x1b".to_vec()),
            '\\' => return Some(b"\x1c".to_vec()),
            ']' => return Some(b"\x1d".to_vec()),
            '^' => return Some(b"\x1e".to_vec()),
            '_' => return Some(b"\x1f".to_vec()),
            '?' => return Some(b"\x7f".to_vec()),
            '@' => return Some(vec![0]),
            _ => {}
        }
    }

    if keystroke.modifiers.alt
        && let Some(ch) = single_char(&keystroke.key)
    {
        let mut bytes = Vec::with_capacity(4);
        bytes.push(0x1b);
        let mut buffer = [0u8; 4];
        bytes.extend_from_slice(ch.encode_utf8(&mut buffer).as_bytes());
        return Some(bytes);
    }

    if let Some(text) = &keystroke.key_char
        && !keystroke.modifiers.control
        && !keystroke.modifiers.alt
    {
        return Some(text.as_bytes().to_vec());
    }

    if let Some(ch) = single_char(&keystroke.key)
        && !keystroke.modifiers.control
    {
        let ch = if keystroke.modifiers.shift {
            ch.to_ascii_uppercase()
        } else {
            ch
        };
        let mut buffer = [0u8; 4];
        return Some(ch.encode_utf8(&mut buffer).as_bytes().to_vec());
    }

    None
}

fn arrow(mode: TermMode, final_byte: u8) -> Vec<u8> {
    if mode.contains(TermMode::APP_CURSOR) {
        vec![0x1b, b'O', final_byte]
    } else {
        vec![0x1b, b'[', final_byte]
    }
}

fn single_char(text: &str) -> Option<char> {
    let mut chars = text.chars();
    match (chars.next(), chars.next()) {
        (Some(ch), None) => Some(ch),
        _ => None,
    }
}

pub fn paste_bytes(mode: TermMode, text: &str) -> Vec<u8> {
    if mode.contains(TermMode::BRACKETED_PASTE) {
        let mut bytes = Vec::with_capacity(text.len() + 12);
        bytes.extend_from_slice(b"\x1b[200~");
        bytes.extend_from_slice(text.as_bytes());
        bytes.extend_from_slice(b"\x1b[201~");
        bytes
    } else {
        text.as_bytes().to_vec()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButtonCode {
    Left,
    Middle,
    Right,
    WheelUp,
    WheelDown,
    WheelLeft,
    WheelRight,
}

impl MouseButtonCode {
    fn base_code(self) -> u8 {
        match self {
            Self::Left => 0,
            Self::Middle => 1,
            Self::Right => 2,
            Self::WheelUp => 64,
            Self::WheelDown => 65,
            Self::WheelLeft => 66,
            Self::WheelRight => 67,
        }
    }

    fn is_wheel(self) -> bool {
        matches!(
            self,
            Self::WheelUp | Self::WheelDown | Self::WheelLeft | Self::WheelRight
        )
    }
}

/// Encode a mouse report. `col`/`row` are zero-based cell coordinates.
pub fn encode_mouse(
    mode: TermMode,
    button: MouseButtonCode,
    pressed: bool,
    motion: bool,
    col: usize,
    row: usize,
) -> Option<Vec<u8>> {
    if !mode.intersects(TermMode::MOUSE_MODE) {
        return None;
    }
    if motion && !mode.intersects(TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION) {
        return None;
    }
    if !pressed && !button.is_wheel() && !mode.contains(TermMode::MOUSE_REPORT_CLICK) {
        return None;
    }

    let mut code = button.base_code();
    if motion {
        code += 32;
    }
    if !pressed && !button.is_wheel() {
        // SGR has an explicit release final byte; legacy mode uses button 3.
        if !mode.contains(TermMode::SGR_MOUSE) {
            code = 3 + if motion { 32 } else { 0 };
        }
    }

    if mode.contains(TermMode::SGR_MOUSE) {
        let final_byte = if pressed || button.is_wheel() {
            'M'
        } else {
            'm'
        };
        Some(format!("\x1b[<{};{};{}{}", code, col + 1, row + 1, final_byte).into_bytes())
    } else {
        let x = (col + 1 + 32).min(255) as u8;
        let y = (row + 1 + 32).min(255) as u8;
        Some(vec![0x1b, b'[', b'M', 32 + code, x, y])
    }
}

pub fn focus_bytes(mode: TermMode, focused: bool) -> Option<Vec<u8>> {
    if !mode.contains(TermMode::FOCUS_IN_OUT) {
        return None;
    }
    Some(if focused {
        b"\x1b[I".to_vec()
    } else {
        b"\x1b[O".to_vec()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(spec: &str) -> Keystroke {
        Keystroke::parse(spec).unwrap()
    }

    #[test]
    fn encodes_basic_keys() {
        assert_eq!(
            keystroke_to_bytes(&key("enter"), TermMode::empty()),
            Some(b"\r".to_vec())
        );
        assert_eq!(
            keystroke_to_bytes(&key("ctrl-c"), TermMode::empty()),
            Some(vec![0x03])
        );
        assert_eq!(
            keystroke_to_bytes(&key("alt-x"), TermMode::empty()),
            Some(b"\x1bx".to_vec())
        );
        assert_eq!(
            keystroke_to_bytes(&key("shift-tab"), TermMode::empty()),
            Some(b"\x1b[Z".to_vec())
        );
    }

    #[test]
    fn arrows_respect_app_cursor() {
        assert_eq!(
            keystroke_to_bytes(&key("up"), TermMode::empty()),
            Some(b"\x1b[A".to_vec())
        );
        assert_eq!(
            keystroke_to_bytes(&key("up"), TermMode::APP_CURSOR),
            Some(b"\x1bOA".to_vec())
        );
    }

    #[test]
    fn bracketed_paste_wraps_text() {
        assert_eq!(
            paste_bytes(TermMode::BRACKETED_PASTE, "a\nb"),
            b"\x1b[200~a\nb\x1b[201~".to_vec()
        );
        assert_eq!(paste_bytes(TermMode::empty(), "a"), b"a".to_vec());
    }

    #[test]
    fn encodes_sgr_mouse() {
        let mode = TermMode::MOUSE_REPORT_CLICK | TermMode::SGR_MOUSE;
        assert_eq!(
            encode_mouse(mode, MouseButtonCode::Left, true, false, 0, 0),
            Some(b"\x1b[<0;1;1M".to_vec())
        );
        assert_eq!(
            encode_mouse(mode, MouseButtonCode::Left, false, false, 4, 2),
            Some(b"\x1b[<0;5;3m".to_vec())
        );
        assert_eq!(
            encode_mouse(mode, MouseButtonCode::WheelUp, true, false, 1, 1),
            Some(b"\x1b[<64;2;2M".to_vec())
        );
    }

    #[test]
    fn focus_only_when_enabled() {
        assert_eq!(focus_bytes(TermMode::empty(), true), None);
        assert_eq!(
            focus_bytes(TermMode::FOCUS_IN_OUT, true),
            Some(b"\x1b[I".to_vec())
        );
    }
}
