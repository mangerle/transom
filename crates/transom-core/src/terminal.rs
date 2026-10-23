use crate::color::Rgb;
use flume::{Receiver, Sender};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::io::{Read, Write};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalSize {
    pub cols: u16,
    pub rows: u16,
}

impl TerminalSize {
    pub const fn new(cols: u16, rows: u16) -> Self {
        Self { cols, rows }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalControl {
    Resize(TerminalSize),
    Close,
}

#[derive(Clone)]
pub struct TerminalControlHandle {
    sender: Sender<TerminalControl>,
}

impl TerminalControlHandle {
    pub fn channel(capacity: usize) -> (Self, Receiver<TerminalControl>) {
        let (sender, receiver) = flume::bounded(capacity.max(1));
        (Self { sender }, receiver)
    }

    pub fn send(&self, control: TerminalControl) -> bool {
        self.sender.send(control).is_ok()
    }

    pub fn resize(&self, size: TerminalSize) -> bool {
        self.send(TerminalControl::Resize(size))
    }

    pub fn close(&self) -> bool {
        self.send(TerminalControl::Close)
    }
}

impl fmt::Debug for TerminalControlHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TerminalControlHandle")
            .finish_non_exhaustive()
    }
}

/// A transport-independent terminal byte stream.
///
/// The terminal widget owns `reader`/`writer`; the transport keeps the receiving
/// half of `control` and reacts to resize and close requests.
pub struct TerminalIo {
    pub reader: Box<dyn Read + Send>,
    pub writer: Box<dyn Write + Send>,
    pub control: TerminalControlHandle,
}

impl TerminalIo {
    pub fn new(
        reader: Box<dyn Read + Send>,
        writer: Box<dyn Write + Send>,
        control: TerminalControlHandle,
    ) -> Self {
        Self {
            reader,
            writer,
            control,
        }
    }
}

impl fmt::Debug for TerminalIo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TerminalIo")
            .field("control", &self.control)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalPalette {
    pub background: Rgb,
    pub foreground: Rgb,
    pub cursor: Rgb,
    pub selection: Rgb,
    pub ansi: [Rgb; 16],
}

impl Default for TerminalPalette {
    fn default() -> Self {
        Self {
            background: Rgb::from_hex(0x1e2127),
            foreground: Rgb::from_hex(0xabb2bf),
            cursor: Rgb::from_hex(0x528bff),
            selection: Rgb::from_hex(0x3e4451),
            ansi: [
                Rgb::from_hex(0x1e2127),
                Rgb::from_hex(0xe06c75),
                Rgb::from_hex(0x98c379),
                Rgb::from_hex(0xe5c07b),
                Rgb::from_hex(0x61afef),
                Rgb::from_hex(0xc678dd),
                Rgb::from_hex(0x56b6c2),
                Rgb::from_hex(0xabb2bf),
                Rgb::from_hex(0x5c6370),
                Rgb::from_hex(0xe06c75),
                Rgb::from_hex(0x98c379),
                Rgb::from_hex(0xe5c07b),
                Rgb::from_hex(0x61afef),
                Rgb::from_hex(0xc678dd),
                Rgb::from_hex(0x56b6c2),
                Rgb::from_hex(0xffffff),
            ],
        }
    }
}

/// 终端默认光标形状。
///
/// 仅作为默认样式：终端应用仍可通过 DECSCUSR 转义序列临时覆盖，
/// 与主流终端的“设置即默认值”行为保持一致。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TerminalCursorShape {
    #[default]
    Block,
    Bar,
    Underline,
}

fn default_true() -> bool {
    true
}

fn default_theme_name() -> String {
    "One Dark".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalProfile {
    /// 为 `None` 时跟随应用默认等宽字体
    pub font_family: Option<String>,
    pub font_size: f32,
    pub line_height: f32,
    pub scrollback: usize,
    pub cursor_blink: bool,
    /// 默认光标形状；应用可通过转义序列覆盖。
    #[serde(default)]
    pub cursor_shape: TerminalCursorShape,
    #[serde(default = "default_true")]
    pub bold_is_bright: bool,
    #[serde(default = "default_true")]
    pub scroll_on_input: bool,
    #[serde(default = "default_theme_name")]
    pub theme_name: String,
    pub palette: TerminalPalette,
}

impl Default for TerminalProfile {
    fn default() -> Self {
        Self {
            font_family: None,
            font_size: 14.0,
            line_height: 1.2,
            scrollback: 10_000,
            cursor_blink: true,
            cursor_shape: TerminalCursorShape::default(),
            bold_is_bright: true,
            scroll_on_input: true,
            theme_name: default_theme_name(),
            palette: TerminalPalette::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct TerminalInteraction {
    pub copy_on_select: bool,
    pub right_click_paste: bool,
    pub multiline_paste_confirm: bool,
    #[serde(default)]
    pub middle_click_paste: bool,
    #[serde(default)]
    pub trim_trailing_whitespace: bool,
}

impl Default for TerminalInteraction {
    fn default() -> Self {
        Self {
            copy_on_select: true,
            right_click_paste: true,
            multiline_paste_confirm: false,
            middle_click_paste: false,
            trim_trailing_whitespace: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_handle_delivers_messages() {
        let (handle, receiver) = TerminalControlHandle::channel(4);
        assert!(handle.resize(TerminalSize::new(120, 40)));
        assert!(handle.close());
        assert_eq!(
            receiver.try_recv().unwrap(),
            TerminalControl::Resize(TerminalSize::new(120, 40))
        );
        assert_eq!(receiver.try_recv().unwrap(), TerminalControl::Close);
    }

    #[test]
    fn default_profile_is_stable() {
        let profile = TerminalProfile::default();
        assert_eq!(profile.scrollback, 10_000);
        assert_eq!(profile.cursor_shape, TerminalCursorShape::Block);
        assert_eq!(profile.palette.ansi.len(), 16);
        assert!(TerminalInteraction::default().copy_on_select);
        assert!(TerminalInteraction::default().right_click_paste);
    }
}
