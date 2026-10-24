use crate::color::Rgb;
use crate::highlight::TerminalHighlight;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::io::{Read, Write};
use tokio::sync::mpsc::{Receiver, Sender};

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
        let (sender, receiver) = tokio::sync::mpsc::channel(capacity.max(1));
        (Self { sender }, receiver)
    }

    /// 同步投递控制消息：快路径无锁 try_send，缓冲满（连续拖拽改尺寸）
    /// 时退化为阻塞反压，与此前 flume 的语义一致。调用方均在 UI 或专用
    /// 线程上下文，不会进入异步运行时，blocking_send 的运行时限制不触发。
    pub fn send(&self, control: TerminalControl) -> bool {
        match self.sender.try_send(control) {
            Ok(()) => true,
            Err(tokio::sync::mpsc::error::TrySendError::Full(control)) => {
                self.sender.blocking_send(control).is_ok()
            }
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => false,
        }
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

/// 与 `alacritty_terminal::term::SEMANTIC_ESCAPE_CHARS` 逐字一致，保证不改设置时行为与内核
/// 口径相同；两者一致性由 transom-terminal 的对照测试兜住（core 不依赖仿真内核，拿不到常量）。
fn default_word_separators() -> String {
    ",│`|:\"' ()[]{}<>\t".to_string()
}

fn default_theme_name() -> String {
    "One Dark".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalProfile {
    /// 为 `None` 时跟随应用默认等宽字体
    pub font_family: Option<String>,
    /// 中日韩等双格字符的专用字体；为 `None` 时交给系统字体回退。
    ///
    /// 为什么要独立配置：等宽西文字体自带的汉字字面宽度往往不等于两格，混排就会错位；
    /// 指定后渲染器会按"恰好两格宽"缩放该字号，让中文对齐由排版保证而不是靠挑选字体。
    #[serde(default)]
    pub cjk_font_family: Option<String>,
    pub font_size: f32,
    pub line_height: f32,
    pub scrollback: usize,
    pub cursor_blink: bool,
    /// 默认光标形状；应用可通过转义序列覆盖。
    #[serde(default)]
    pub cursor_shape: TerminalCursorShape,
    /// 双击选词时的分隔字符：命中其中任意一个即断词。
    ///
    /// 放在 profile 而不是 interaction：断词是仿真内核算的（`SelectionType::Semantic`
    /// 由 alacritty_terminal 的 `Config::semantic_escape_chars` 驱动），不是视图层行为。
    /// 刻意不做 trim：空格本身就是合法分隔符，去掉首尾空格会让"只按空格断词"无法表达。
    #[serde(default = "default_word_separators")]
    pub word_separators: String,
    #[serde(default = "default_true")]
    pub bold_is_bright: bool,
    #[serde(default = "default_true")]
    pub scroll_on_input: bool,
    /// 关键词规则集自动高亮。
    #[serde(default)]
    pub highlight: TerminalHighlight,
    #[serde(default = "default_theme_name")]
    pub theme_name: String,
    pub palette: TerminalPalette,
}

impl Default for TerminalProfile {
    fn default() -> Self {
        Self {
            font_family: None,
            cjk_font_family: None,
            font_size: 14.0,
            line_height: 1.2,
            scrollback: 10_000,
            cursor_blink: true,
            cursor_shape: TerminalCursorShape::default(),
            word_separators: default_word_separators(),
            bold_is_bright: true,
            scroll_on_input: true,
            theme_name: default_theme_name(),
            highlight: TerminalHighlight::default(),
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
        let (handle, mut receiver) = TerminalControlHandle::channel(4);
        assert!(handle.resize(TerminalSize::new(120, 40)));
        assert!(handle.close());
        assert_eq!(
            receiver.try_recv().unwrap(),
            TerminalControl::Resize(TerminalSize::new(120, 40))
        );
        assert_eq!(receiver.try_recv().unwrap(), TerminalControl::Close);
    }

    /// 分隔符属仿真内核配置：老配置没有这个字段时要退回内核默认，而不是加载失败。
    #[test]
    fn word_separators_default_and_round_trip() {
        let default_separators = TerminalProfile::default().word_separators.clone();
        assert!(default_separators.contains(','));
        assert!(default_separators.contains(' '), "空格默认应断词");
        assert!(default_separators.contains('('));
        let profile = TerminalProfile {
            word_separators: "-/.".into(),
            ..TerminalProfile::default()
        };
        let json = serde_json::to_string(&profile).unwrap();
        let decoded: TerminalProfile = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.word_separators, "-/.");

        let mut value = serde_json::to_value(&profile).unwrap();
        value.as_object_mut().unwrap().remove("word_separators");
        let legacy: TerminalProfile = serde_json::from_value(value).unwrap();
        assert_eq!(
            legacy.word_separators,
            TerminalProfile::default().word_separators
        );
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

    /// 双重字体是后加的字段：老的会话与设置文件里没有它，必须按"跟随系统回退"降级，
    /// 而不是让整份配置加载失败。
    #[test]
    fn profile_without_cjk_font_still_loads() {
        let profile = TerminalProfile {
            cjk_font_family: Some("Sarasa Mono SC".into()),
            ..TerminalProfile::default()
        };
        let mut value = serde_json::to_value(&profile).expect("默认配置应可序列化");
        value
            .as_object_mut()
            .expect("配置序列化为对象")
            .remove("cjk_font_family");
        let decoded: TerminalProfile =
            serde_json::from_value(value).expect("缺少 cjk_font_family 的旧配置应能解析");
        assert_eq!(decoded.cjk_font_family, None);
    }

    /// 双重字体字段必须能原样往返，否则用户在设置里选的中文字体存不住。
    #[test]
    fn cjk_font_round_trips() {
        let profile = TerminalProfile {
            cjk_font_family: Some("更纱黑体 Mono SC".into()),
            ..TerminalProfile::default()
        };
        let json = serde_json::to_string(&profile).unwrap();
        let decoded: TerminalProfile = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.cjk_font_family.as_deref(), Some("更纱黑体 Mono SC"));
    }
}
