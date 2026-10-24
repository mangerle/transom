//! 内置 UI 主题资产：编译期嵌入，启动时一次性注册进主题注册表。

/// 内置主题集合（ThemeSet 格式）。
///
/// 刻意不携带 `font.size` / `mono_font.size`：全局字号的唯一权威是密度档位
/// （`UiDensity`），主题文件一旦携带字号，`apply_config` 会在每次应用或系统
/// 明暗联动时覆盖运行期密度设置。
///
/// 命名避开 gpui-kit 自带的 "Default"：注册表对重复名静默跳过，同名主题将
/// 永远注册不进去。
pub(crate) const BUILTIN_THEMES: &str = include_str!("themes/transom_dark.json");
