use crate::model::terminal::TerminalPalette;
use serde::{Deserialize, Serialize};

mod presets;
pub use presets::preset_themes;

/// 调色板明暗属性。
///
/// 为什么放在 core 而不是 UI 层：设置页要判断"界面主题与终端配色的明暗反差"
/// 并给出提示，而 core 不允许引入 gpui 依赖，因此用自定义二值枚举表达。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PaletteMode {
    /// 浅色终端配色。
    Light,
    /// 深色终端配色。
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

    /// 指定明暗属性的链式构造，便于测试或外部导入构造浅色方案。
    pub fn with_mode(mut self, mode: PaletteMode) -> Self {
        self.mode = mode;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 旧版本序列化数据没有 mode 字段：反序列化必须按深色降级而不是失败。
    #[test]
    fn preset_theme_without_mode_defaults_to_dark() {
        let legacy = PresetTheme::new("旧数据", TerminalPalette::default());
        let mut value = serde_json::to_value(&legacy).unwrap();
        value.as_object_mut().unwrap().remove("mode");
        let theme: PresetTheme = serde_json::from_value(value).unwrap();
        assert_eq!(theme.mode, PaletteMode::Dark);
    }
}
