use crate::color::Rgb;
use crate::terminal::TerminalPalette;
use serde::{Deserialize, Serialize};

/// 调色板明暗属性。
///
/// 为什么放在 core 而不是 UI 层：设置页要判断"界面主题与终端配色的明暗反差"
/// 并给出提示，而 core 不允许引入 gpui 依赖，因此用自定义二值枚举表达。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PaletteMode {
    /// 浅色终端配色。
    Light,
    /// 深色终端配色；现有全部内置预设都是深色。
    #[default]
    Dark,
}

/// 预设配色方案
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresetTheme {
    /// 配色方案唯一名称
    pub name: String,
    /// 配色明暗属性；旧版本序列化数据缺少该字段时按深色处理。
    #[serde(default)]
    pub mode: PaletteMode,
    /// 调色板
    pub palette: TerminalPalette,
}

impl PresetTheme {
    pub fn new(name: impl Into<String>, palette: TerminalPalette) -> Self {
        Self {
            name: name.into(),
            mode: PaletteMode::Dark,
            palette,
        }
    }

    /// 指定明暗属性的链式构造；内置预设目前全为深色，供将来新增浅色预设使用。
    pub fn with_mode(mut self, mode: PaletteMode) -> Self {
        self.mode = mode;
        self
    }
}

/// 获取系统内置的经典终端配色方案列表
pub fn preset_themes() -> Vec<PresetTheme> {
    vec![
        // 1. One Dark（经典暗色）
        PresetTheme::new("One Dark", TerminalPalette::default()),
        // 2. Dracula（吸血鬼高对比度）
        PresetTheme::new(
            "Dracula",
            TerminalPalette {
                background: Rgb::from_hex(0x282a36),
                foreground: Rgb::from_hex(0xf8f8f2),
                cursor: Rgb::from_hex(0xf8f8f2),
                selection: Rgb::from_hex(0x44475a),
                ansi: [
                    Rgb::from_hex(0x21222c), // 黑色
                    Rgb::from_hex(0xff5555), // 红色
                    Rgb::from_hex(0x50fa7b), // 绿色
                    Rgb::from_hex(0xf1fa8c), // 黄色
                    Rgb::from_hex(0xbd93f9), // 蓝色
                    Rgb::from_hex(0xff79c6), // 紫色
                    Rgb::from_hex(0x8be9fd), // 青色
                    Rgb::from_hex(0xf8f8f2), // 白色
                    Rgb::from_hex(0x6272a4), // 明亮黑（灰色）
                    Rgb::from_hex(0xff6e6e), // 明亮红
                    Rgb::from_hex(0x69ff94), // 明亮绿
                    Rgb::from_hex(0xffffa5), // 明亮黄
                    Rgb::from_hex(0xd6acff), // 明亮蓝
                    Rgb::from_hex(0xff92df), // 明亮紫
                    Rgb::from_hex(0xa4ffff), // 明亮青
                    Rgb::from_hex(0xffffff), // 明亮白
                ],
            },
        ),
        // 3. Monokai Pro
        PresetTheme::new(
            "Monokai Pro",
            TerminalPalette {
                background: Rgb::from_hex(0x2d2a2e),
                foreground: Rgb::from_hex(0xfcfcfa),
                cursor: Rgb::from_hex(0xfcfcfa),
                selection: Rgb::from_hex(0x403e41),
                ansi: [
                    Rgb::from_hex(0x2d2a2e),
                    Rgb::from_hex(0xff6188),
                    Rgb::from_hex(0xa9dc76),
                    Rgb::from_hex(0xffd866),
                    Rgb::from_hex(0x78dce8),
                    Rgb::from_hex(0xab9df2),
                    Rgb::from_hex(0x78dce8),
                    Rgb::from_hex(0xfcfcfa),
                    Rgb::from_hex(0x727072),
                    Rgb::from_hex(0xff6188),
                    Rgb::from_hex(0xa9dc76),
                    Rgb::from_hex(0xffd866),
                    Rgb::from_hex(0x78dce8),
                    Rgb::from_hex(0xab9df2),
                    Rgb::from_hex(0x78dce8),
                    Rgb::from_hex(0xffffff),
                ],
            },
        ),
        // 4. Nord（极地冷色系）
        PresetTheme::new(
            "Nord",
            TerminalPalette {
                background: Rgb::from_hex(0x2e3440),
                foreground: Rgb::from_hex(0xd8dee9),
                cursor: Rgb::from_hex(0xd8dee9),
                selection: Rgb::from_hex(0x434c5e),
                ansi: [
                    Rgb::from_hex(0x3b4252),
                    Rgb::from_hex(0xbf616a),
                    Rgb::from_hex(0xa3be8c),
                    Rgb::from_hex(0xebcb8b),
                    Rgb::from_hex(0x81a1c1),
                    Rgb::from_hex(0xb48ead),
                    Rgb::from_hex(0x88c0d0),
                    Rgb::from_hex(0xe5e9f0),
                    Rgb::from_hex(0x4c566a),
                    Rgb::from_hex(0xbf616a),
                    Rgb::from_hex(0xa3be8c),
                    Rgb::from_hex(0xebcb8b),
                    Rgb::from_hex(0x81a1c1),
                    Rgb::from_hex(0xb48ead),
                    Rgb::from_hex(0x8fbcbb),
                    Rgb::from_hex(0xeceff4),
                ],
            },
        ),
        // 5. Solarized Dark
        PresetTheme::new(
            "Solarized Dark",
            TerminalPalette {
                background: Rgb::from_hex(0x002b36),
                foreground: Rgb::from_hex(0x839496),
                cursor: Rgb::from_hex(0x93a1a1),
                selection: Rgb::from_hex(0x073642),
                ansi: [
                    Rgb::from_hex(0x073642),
                    Rgb::from_hex(0xdc322f),
                    Rgb::from_hex(0x859900),
                    Rgb::from_hex(0xb58900),
                    Rgb::from_hex(0x268bd2),
                    Rgb::from_hex(0xd33682),
                    Rgb::from_hex(0x2aa198),
                    Rgb::from_hex(0xeee8d5),
                    Rgb::from_hex(0x002b36),
                    Rgb::from_hex(0xcb4b16),
                    Rgb::from_hex(0x586e75),
                    Rgb::from_hex(0x657b83),
                    Rgb::from_hex(0x839496),
                    Rgb::from_hex(0x6c71c4),
                    Rgb::from_hex(0x93a1a1),
                    Rgb::from_hex(0xfdf6e3),
                ],
            },
        ),
        // 6. Gruvbox Dark
        PresetTheme::new(
            "Gruvbox Dark",
            TerminalPalette {
                background: Rgb::from_hex(0x282828),
                foreground: Rgb::from_hex(0xebdbb2),
                cursor: Rgb::from_hex(0xebdbb2),
                selection: Rgb::from_hex(0x3c3836),
                ansi: [
                    Rgb::from_hex(0x282828),
                    Rgb::from_hex(0xcc241d),
                    Rgb::from_hex(0x98971a),
                    Rgb::from_hex(0xd79921),
                    Rgb::from_hex(0x458588),
                    Rgb::from_hex(0xb16286),
                    Rgb::from_hex(0x689d6a),
                    Rgb::from_hex(0xa89984),
                    Rgb::from_hex(0x928374),
                    Rgb::from_hex(0xfb4934),
                    Rgb::from_hex(0xb8bb26),
                    Rgb::from_hex(0xfabd2f),
                    Rgb::from_hex(0x83a598),
                    Rgb::from_hex(0xd3869b),
                    Rgb::from_hex(0x8ec07c),
                    Rgb::from_hex(0xebdbb2),
                ],
            },
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_preset_themes_validity() {
        let themes = preset_themes();
        assert!(themes.len() >= 6);
        for theme in themes {
            assert!(!theme.name.is_empty());
            assert_eq!(theme.palette.ansi.len(), 16);
            // 现有内置预设全部是深色配色，mode 缺省即为深色。
            assert_eq!(theme.mode, PaletteMode::Dark);
        }
    }

    /// 旧版本序列化数据没有 mode 字段：反序列化必须按深色降级而不是失败。
    #[test]
    fn preset_theme_without_mode_defaults_to_dark() {
        // 调色板字段是必填的，先构造一份合法值再去掉 mode 模拟旧数据。
        let legacy = PresetTheme::new("旧数据", TerminalPalette::default());
        let mut value = serde_json::to_value(&legacy).unwrap();
        value.as_object_mut().unwrap().remove("mode");
        let theme: PresetTheme = serde_json::from_value(value).unwrap();
        assert_eq!(theme.mode, PaletteMode::Dark);
    }
}
