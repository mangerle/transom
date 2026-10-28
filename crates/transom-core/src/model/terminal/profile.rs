//! 终端配置方案、调色板与交互行为。

use super::highlight::TerminalHighlight;
use super::key_bindings::TerminalKeyBinding;
use crate::color::Rgb;
use serde::{Deserialize, Serialize};

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
