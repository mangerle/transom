//! 内置终端配色预设。
//!
//! 配色数据存放在 JSON 中而不是逐个手写结构体：同一份数据要同时供终端设置页、
//! 命令面板与 UI 主题跟随逻辑使用，声明式数据能避免三处名称漂移。解析只在首次
//! 调用时执行一次，之后返回克隆给调用方，保持既有 `Vec<PresetTheme>` 接口不变。

use super::{PaletteMode, PresetTheme};
use crate::model::color::Rgb;
use crate::model::terminal::TerminalPalette;
use serde::Deserialize;
use std::sync::OnceLock;

const PRESETS_JSON: &str = include_str!("presets.json");

#[derive(Debug, Deserialize)]
struct PresetDocument {
    themes: Vec<PresetDocumentTheme>,
}

#[derive(Debug, Deserialize)]
struct PresetDocumentTheme {
    name: String,
    mode: PaletteMode,
    background: String,
    foreground: String,
    #[serde(default)]
    foreground_intense: Option<String>,
    cursor: String,
    selection: String,
    line_highlight: String,
    find_match_background: String,
    find_match_border: String,
    ansi: Vec<String>,
}

impl PresetDocumentTheme {
    fn parse(self) -> PresetTheme {
        PresetTheme {
            name: self.name,
            mode: self.mode,
            palette: TerminalPalette {
                background: parse_required_color(&self.background),
                foreground: parse_required_color(&self.foreground),
                foreground_intense: self.foreground_intense.as_deref().map(parse_required_color),
                cursor: parse_required_color(&self.cursor),
                selection: parse_required_color(&self.selection),
                line_highlight: parse_required_color(&self.line_highlight),
                find_match_background: parse_required_color(&self.find_match_background),
                find_match_border: parse_required_color(&self.find_match_border),
                ansi: parse_ansi(self.ansi),
            },
        }
    }
}

/// 把内置数据中的颜色字符串转成强类型 RGB。
///
/// 内置 JSON 的合法性由单元测试兜底；这里保留 `expect` 是为了让公开的
/// `preset_themes()` 保持无 I/O、无运行期错误分支的简洁契约。若将来允许
/// 从用户目录加载预设，必须改为返回强类型错误，不能复用这条内置路径。
fn parse_required_color(value: &str) -> Rgb {
    Rgb::parse_hex(value).expect("内置终端预设颜色必须是合法十六进制值")
}

fn parse_ansi(values: Vec<String>) -> [Rgb; 16] {
    let mut ansi = [Rgb::new(0, 0, 0); 16];
    for (slot, value) in values.iter().enumerate().take(16) {
        ansi[slot] = parse_required_color(value);
    }
    ansi
}

fn load_presets() -> Vec<PresetTheme> {
    let document: PresetDocument =
        serde_json::from_str(PRESETS_JSON).expect("内置终端预设 JSON 必须可以反序列化");
    document
        .themes
        .into_iter()
        .map(PresetDocumentTheme::parse)
        .collect()
}

/// 获取内置终端配色方案列表。
///
/// 返回顺序与设置页展示顺序一致；深色、浅色与 JetBrains Islands 终端配色都在
/// 同一张表中，调用方不需要再维护第二套名字映射。
pub fn preset_themes() -> Vec<PresetTheme> {
    static THEMES: OnceLock<Vec<PresetTheme>> = OnceLock::new();
    THEMES.get_or_init(load_presets).clone()
}

#[cfg(test)]
mod tests {
    use super::preset_themes;
    use crate::model::theme::PaletteMode;

    #[test]
    fn bundled_terminal_presets_are_complete() {
        let themes = preset_themes();
        assert_eq!(themes.len(), 23, "内置终端预设数量应与数据文件一致");
        assert!(
            themes.iter().any(|theme| theme.mode == PaletteMode::Light),
            "必须包含浅色终端预设"
        );
        assert!(
            themes
                .iter()
                .any(|theme| theme.name == "JetBrains Islands Dark"),
            "必须显式提供 JetBrains Islands 深色终端配色"
        );
        assert!(
            themes
                .iter()
                .any(|theme| theme.name == "JetBrains Islands Light"),
            "必须显式提供 JetBrains Islands 浅色终端配色"
        );
        for theme in themes {
            assert!(!theme.name.trim().is_empty());
            assert_eq!(theme.palette.ansi.len(), 16);
            assert_ne!(theme.palette.background, theme.palette.foreground);
        }
    }

    #[test]
    fn terminal_preset_names_are_unique() {
        let themes = preset_themes();
        let mut names: Vec<_> = themes.iter().map(|theme| theme.name.as_str()).collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(names.len(), before);
    }
}
