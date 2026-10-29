//! 内置 UI 主题资产：编译期嵌入，启动时一次性注册进主题注册表。
//!
//! # 编辑器选区是"预补偿"色值
//! gpui-component 在应用主题时把 `selection.background` 的 alpha 统一钳到
//! 0.3（`schema.rs` 的 `clamp_alpha`）。主题文件里写的选区色不是最终显示色，
//! 而是与编辑器背景按 3:7 合成之前的基色：浅色主题写深蓝 `#0033cc`（合成
//! `#b2c2f0`，对比 1.77），深色主题写浅蓝灰 `#c8d4ee`（合成约 `#515763`，
//! 对比 2.2）。这已经是在 0.3 上限内能做到的最强档；不要按字面值判断观感，
//! 也不要把原主题的 `#a6d2ff`/`#214283` 改回来——它们在 0.3 下几乎不可见。
//! 彻底解除限制需要上游把上限改成可配置项。
//!
//! # 编辑器辨识度
//! `editor.gutter.background` 与 `editor.background` 刻意拉开一档，让行号槽
//! 形成竖向分层；`editor.active_line.background` 从原生几乎与背景同色提到
//! 可辨识的档位；`editor.line_number` 提到 3:1 以上，避免小字行号过于发虚。

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

/// 全部内置主题集合，启动时按顺序注册（内存注入，消除热重载前的首帧竞态）。
pub(crate) const ALL: &[&str] = &[TRANSMOM_DARK, JETBRAINS_ISLANDS];

/// 内置主题的受管落盘清单：(目录内文件名, 内容)。
///
/// 主题注册表的热重载以"目录全量重建"为语义（reload 清空注册表后只装回
/// gpui-kit 内置 Default 与目录文件），纯内存注册的内置主题会在目录变动后
/// 被清掉，因此内置主题同时以受管文件形式落盘，与用户主题在同一语义下共存。
pub(crate) const MANAGED_FILES: &[(&str, &str)] = &[
    ("transom-dark.json", TRANSMOM_DARK),
    ("jetbrains-islands.json", JETBRAINS_ISLANDS),
];
