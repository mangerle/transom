//! 内置 UI 主题资产：编译期嵌入，启动时一次性注册进主题注册表。

/// 内置主题集合（ThemeSet 格式）：与 One Dark 终端预设配套的出厂深色。
///
/// 刻意不携带 `font.size` / `mono_font.size`：全局字号的唯一权威是密度档位
/// （`UiDensity`），主题文件一旦携带字号，`apply_config` 会在每次应用或系统
/// 明暗联动时覆盖运行期密度设置。
///
/// 命名避开 gpui-kit 自带的 "Default"：注册表对重复名静默跳过，同名主题将
/// 永远注册不进去。
pub(crate) const TRANSMOM_DARK: &str = include_str!("themes/transom_dark.json");

/// JetBrains Islands 主题对（Dark/Light），与 Zed 编辑器同名主题同源。
///
/// 颜色数据自 [artemevsevev/zed-theme-jetbrains](https://github.com/artemevsevev/zed-theme-jetbrains)
/// （Zed 主题，Apache License 2.0，Copyright Artem Evsevev）移植：Zed 的
/// `style.colors` 语义键映射为本应用的组件级颜色 token，`style.syntax` 按
/// gpui-kit 兼容 Zed 的 highlight 节原样携带（含斜体/粗体）。两个主题名字
/// 以 " Dark"/" Light" 结尾构成一族，明暗切换时 [`super::resolve`] 自动联动。
pub(crate) const JETBRAINS_ISLANDS: &str = include_str!("themes/jetbrains_islands.json");

/// 全部内置主题集合，启动时按顺序注册。
pub(crate) const ALL: &[&str] = &[TRANSMOM_DARK, JETBRAINS_ISLANDS];
