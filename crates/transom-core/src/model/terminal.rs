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

/// 终端配色跟随界面主题的哨兵值。
///
/// 主题 JSON 可携带伴生的终端调色板（`terminal` 节），终端配色默认随 UI 主题
/// 明暗联动；设置里显式选了预设（"One Dark" 等）即覆盖跟随。
pub const FOLLOW_UI_THEME: &str = "follow-ui";

fn default_theme_name() -> String {
    FOLLOW_UI_THEME.to_string()
}

/// 终端自定义按键映射：把一组按键组合直接转换成原始字节序列。
///
/// 设计原理（Why）：
/// - 终端程序（尤其是远程 TUI）需要一些本地键盘发不出的序列，例如
///   `Ctrl+Shift+Left` 在 tmux/vim 里应发 `\e[1;6D`；让用户自己配置比在
///   代码里硬编码某套终端约定更可靠；
/// - `keys` 使用 gpui 的按键语法（`ctrl-shift-left`、`alt-backspace`、`f1`），
///   `sequence` 支持 `\e`、`\x1b`、`\n`、`\r`、`\t`、`\\`、`\0` 与 `\u{...}`；
/// - 校验放在设置保存时一次性完成，运行期只做查表匹配，键盘热路径不做解析。
///
/// 代价与局限：映射在应用快捷键之后、终端默认编码之前生效；与全局应用
/// 快捷键冲突时以全局快捷键为准（避免用户把设置页锁死）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalKeyBinding {
    /// gpui 按键语法，如 `ctrl-shift-left`。
    pub keys: String,
    /// 要发送的原始字节序列（支持反斜杠转义）。
    pub sequence: String,
}

impl TerminalKeyBinding {
    /// 解析多行配置，每行形如 `ctrl-shift-left = \e[1;6D`。
    ///
    /// 空行与 `#` 开头的注释忽略；错误信息带行号，便于在设置页定位。
    pub fn parse_multiline(text: &str) -> Result<Vec<Self>, String> {
        let mut bindings = Vec::new();
        for (index, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((keys, sequence)) = line.split_once('=') else {
                return Err(format!(
                    "第 {} 行缺少 \"=\"，格式应为：按键 = 转义序列",
                    index + 1
                ));
            };
            let keys = keys.trim();
            let sequence = sequence.trim();
            if keys.is_empty() {
                return Err(format!("第 {} 行按键为空", index + 1));
            }
            if sequence.is_empty() {
                return Err(format!("第 {} 行转义序列为空", index + 1));
            }
            decode_escape_sequence(sequence)
                .map_err(|error| format!("第 {} 行转义序列非法: {error}", index + 1))?;
            bindings.push(Self {
                keys: keys.to_string(),
                sequence: sequence.to_string(),
            });
        }
        Ok(bindings)
    }

    /// 序列化为多行配置文本；与 [`Self::parse_multiline`] 互逆。
    pub fn to_multiline(bindings: &[Self]) -> String {
        bindings
            .iter()
            .map(|binding| format!("{} = {}", binding.keys, binding.sequence))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// 解码为实际发送的字节；供终端视图与设置校验共用。
    ///
    /// # Errors
    /// 转义不合法、`\x` 后不是两位十六进制或序列为空时返回中文原因。
    pub fn decode_sequence(&self) -> Result<Vec<u8>, String> {
        decode_escape_sequence(&self.sequence)
    }
}

/// 解码带反斜杠转义的字节序列。
fn decode_escape_sequence(input: &str) -> Result<Vec<u8>, String> {
    fn hex_value(byte: u8) -> Option<u8> {
        match byte {
            b'0'..=b'9' => Some(byte - b'0'),
            b'a'..=b'f' => Some(byte - b'a' + 10),
            b'A'..=b'F' => Some(byte - b'A' + 10),
            _ => None,
        }
    }
    let mut bytes = Vec::with_capacity(input.len());
    let mut chars = input.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            let mut buffer = [0u8; 4];
            bytes.extend_from_slice(ch.encode_utf8(&mut buffer).as_bytes());
            continue;
        }
        let Some(escaped) = chars.next() else {
            return Err("序列以单个反斜杠结尾".to_string());
        };
        match escaped {
            'e' | 'E' => bytes.push(0x1b),
            'n' => bytes.push(b'\n'),
            'r' => bytes.push(b'\r'),
            't' => bytes.push(b'\t'),
            '0' => bytes.push(0),
            '\\' => bytes.push(b'\\'),
            'x' | 'X' => {
                let Some(high) = chars.next().and_then(|ch| hex_value(ch as u8)) else {
                    return Err("\\x 后必须跟两位十六进制数字".to_string());
                };
                let Some(low) = chars.next().and_then(|ch| hex_value(ch as u8)) else {
                    return Err("\\x 后必须跟两位十六进制数字".to_string());
                };
                bytes.push((high << 4) | low);
            }
            'u' => {
                let Some('{') = chars.next() else {
                    return Err("\\u 后必须跟 {码点}，例如 \\u{1b}".to_string());
                };
                let mut value = 0u32;
                let mut digits = 0usize;
                let mut closed = false;
                for ch in chars.by_ref() {
                    if ch == '}' {
                        closed = true;
                        break;
                    }
                    let Some(digit) = hex_value(ch as u8) else {
                        return Err("\\u{...} 内只能是十六进制数字".to_string());
                    };
                    digits += 1;
                    if digits > 6 {
                        return Err("\\u{...} 码点过长".to_string());
                    }
                    value = value.saturating_mul(16).saturating_add(u32::from(digit));
                }
                if !closed || digits == 0 {
                    return Err("\\u{...} 未闭合或为空".to_string());
                }
                let Some(ch) = char::from_u32(value) else {
                    return Err(format!("\\u{{{value:x}}} 不是合法 Unicode 码点"));
                };
                let mut buffer = [0u8; 4];
                bytes.extend_from_slice(ch.encode_utf8(&mut buffer).as_bytes());
            }
            other => {
                return Err(format!(
                    "不支持的转义 \\{other}，可用 \\e、\\x1b、\\n、\\r、\\t、\\\\、\\0、\\u{{1b}}"
                ));
            }
        }
    }
    if bytes.is_empty() {
        return Err("转义序列不能为空".to_string());
    }
    Ok(bytes)
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
    /// 备屏（vim/htop 等）下的滚轮语义：开启时滚轮转换为方向键发送给程序
    /// （Zed 的 alternate_scroll 语义）；关闭时滚轮在备屏下不起作用。
    /// 鼠标上报模式（MOUSE_MODE）不受此设置影响——那时滚轮本身就是事件。
    #[serde(default = "default_true")]
    pub alternate_scroll: bool,
    /// 关键词规则集自动高亮。
    #[serde(default)]
    pub highlight: TerminalHighlight,
    #[serde(default = "default_theme_name")]
    pub theme_name: String,
    pub palette: TerminalPalette,
    /// 终端背景透明度（百分比）：0 表示完全不透明，100 表示完全透明。
    ///
    /// 折算成背景色的 alpha，透出下层应用背景；显式设了背景色的单元格仍保持
    /// 不透明，否则整块半透明会连同选区与彩色输出一起洗白。
    /// 该值由窗口外观设置下发（窗口整体透明），界面没有独立的终端级入口。
    #[serde(default)]
    pub background_transparency: u8,
    /// 自定义按键映射；老配置没有该字段时为空。
    #[serde(default)]
    pub key_bindings: Vec<TerminalKeyBinding>,
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
            alternate_scroll: true,
            theme_name: default_theme_name(),
            highlight: TerminalHighlight::default(),
            palette: TerminalPalette::default(),
            background_transparency: 0,
            key_bindings: Vec::new(),
        }
    }
}

/// 终端右键动作档位。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RightClickBehavior {
    /// 右键直接粘贴剪贴板，不弹菜单（老配置 `right_click_paste = true`）。
    #[default]
    Paste,
    /// 右键只弹上下文菜单（老配置 `right_click_paste = false`）。
    Menu,
    /// 右键弹菜单，菜单首项提供"粘贴"。
    MenuWithPaste,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct TerminalInteraction {
    pub copy_on_select: bool,
    /// 旧版右键粘贴开关；仅为兼容老配置保留，读写以 `right_click` 为准。
    ///
    /// 反序列化后由 [`TerminalInteraction::right_click_behavior`] 迁移：
    /// `true → Paste`、`false → Menu`，不丢失用户此前的选择。
    pub right_click_paste: bool,
    /// 右键动作三档；老配置没有该字段时按旧开关迁移。
    #[serde(default)]
    pub right_click: Option<RightClickBehavior>,
    pub multiline_paste_confirm: bool,
    #[serde(default)]
    pub middle_click_paste: bool,
    #[serde(default)]
    pub trim_trailing_whitespace: bool,
}

impl TerminalInteraction {
    /// 当前生效的右键动作；老配置按旧布尔开关迁移。
    pub fn right_click_behavior(&self) -> RightClickBehavior {
        self.right_click.unwrap_or(if self.right_click_paste {
            RightClickBehavior::Paste
        } else {
            RightClickBehavior::Menu
        })
    }

    /// 设置右键档位，并同步旧字段，避免降级版本读到相反语义。
    pub fn set_right_click_behavior(&mut self, behavior: RightClickBehavior) {
        self.right_click = Some(behavior);
        self.right_click_paste = behavior == RightClickBehavior::Paste;
    }
}

impl Default for TerminalInteraction {
    fn default() -> Self {
        Self {
            copy_on_select: true,
            right_click_paste: true,
            right_click: Some(RightClickBehavior::Paste),
            multiline_paste_confirm: false,
            middle_click_paste: false,
            trim_trailing_whitespace: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 老配置没有 `right_click` 字段：必须按旧开关迁移，不能把用户此前的
    /// "右键粘贴"悄悄改成弹菜单。
    #[test]
    fn legacy_right_click_flag_migrates_to_behavior() {
        let paste: TerminalInteraction = serde_json::from_str(
            r#"{"copy_on_select":true,"right_click_paste":true,"multiline_paste_confirm":false}"#,
        )
        .unwrap();
        assert_eq!(paste.right_click_behavior(), RightClickBehavior::Paste);

        let menu: TerminalInteraction = serde_json::from_str(
            r#"{"copy_on_select":true,"right_click_paste":false,"multiline_paste_confirm":false}"#,
        )
        .unwrap();
        assert_eq!(menu.right_click_behavior(), RightClickBehavior::Menu);
    }

    /// 新档位与旧布尔字段双向同步：老版本降级读取时语义不反转。
    #[test]
    fn setting_behavior_updates_legacy_flag() {
        let mut interaction = TerminalInteraction::default();
        interaction.set_right_click_behavior(RightClickBehavior::MenuWithPaste);
        assert!(!interaction.right_click_paste);
        assert_eq!(
            interaction.right_click_behavior(),
            RightClickBehavior::MenuWithPaste
        );
        interaction.set_right_click_behavior(RightClickBehavior::Paste);
        assert!(interaction.right_click_paste);
    }

    /// 转义序列解码覆盖终端最常用的几种写法。
    #[test]
    fn key_binding_decodes_escape_sequences() {
        let binding = TerminalKeyBinding {
            keys: "ctrl-shift-left".into(),
            sequence: "\\e[1;6D".into(),
        };
        assert_eq!(binding.decode_sequence().unwrap(), b"\x1b[1;6D");

        let bytes = decode_escape_sequence("\\x1b]0;title\\x07").unwrap();
        assert_eq!(bytes, b"\x1b]0;title\x07");
        assert_eq!(decode_escape_sequence("A\\nB").unwrap(), b"A\nB");
        assert_eq!(decode_escape_sequence("中").unwrap(), "中".as_bytes());
        assert_eq!(
            decode_escape_sequence("\\u{4e2d}").unwrap(),
            "中".as_bytes()
        );
        assert!(decode_escape_sequence("\\").is_err());
        assert!(decode_escape_sequence("\\x1").is_err());
        assert!(decode_escape_sequence("\\q").is_err());
        assert!(decode_escape_sequence("").is_err());
    }

    /// 多行配置忽略空行与注释，错误带行号；往返序列化保持一致。
    #[test]
    fn key_bindings_parse_multiline_round_trip() {
        let text = "# tmux/vim 常用序列\nctrl-shift-left = \\e[1;6D\n\nalt-backspace = \\x7f\n";
        let bindings = TerminalKeyBinding::parse_multiline(text).unwrap();
        assert_eq!(bindings.len(), 2);
        assert_eq!(bindings[0].keys, "ctrl-shift-left");
        assert_eq!(
            TerminalKeyBinding::to_multiline(&bindings),
            "ctrl-shift-left = \\e[1;6D\nalt-backspace = \\x7f"
        );

        let error = TerminalKeyBinding::parse_multiline("ctrl-a \\e").unwrap_err();
        assert!(error.contains("第 1 行"), "错误: {error}");
        let error = TerminalKeyBinding::parse_multiline("ctrl-a = ").unwrap_err();
        assert!(error.contains("第 1 行"), "错误: {error}");
    }

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
