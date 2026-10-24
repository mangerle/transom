//! UI 主题：内置主题注册、外观偏好解析与全局主题应用。
//!
//! # 唯一写路径
//! 所有对 gpui 全局 `Theme` 的修改必须经本模块的 [`apply_appearance`]（内部走
//! `Theme::update`，自动完成 colors/tokens 对账、同步 gpui-base 投影并刷新全部
//! 窗口）。严禁绕行 `Theme::global_mut`：它不同步 tokens 与底层投影、也不触发
//! 重绘，必然出现"部分控件新配色、部分旧配色"的半态。
//!
//! # 切换序列
//! `apply_config` 只写自己 mode 对应的槽位（见 gpui-component `schema.rs`），因此
//! 必须在同一个 `Theme::update` 闭包内成对补齐另一槽位：系统明暗变化经
//! `Theme::change` 重放当前槽位时，浅/深两个模式都要有主题可用，这正是"跟随
//! 系统"互切零额外成本的前提。
//!
//! # 字号权威
//! `theme.font_size` / `mono_font_size` 的唯一权威是 [`UiDensity`]。内置主题 JSON
//! 一律不携带 `font.size` / `mono_font.size`——`apply_config` 遇到主题文件携带
//! 字号时会覆盖运行期密度，系统明暗联动也会静默重置用户选择。因此字号在
//! `apply_config` 之后于同一闭包内写入。

mod builtin;
pub(crate) mod tokens;

use gpui_kit::component::theme::{Theme, ThemeConfig, ThemeMode, ThemeRegistry};
use gpui_kit::*;
use std::rc::Rc;
use transom_core::{AppearanceSettings, UiThemeMode};

/// 解析结果：当前生效主题与浅/深两槽位的兜底配置。
struct ResolvedThemes {
    active: Rc<ThemeConfig>,
    light: Rc<ThemeConfig>,
    dark: Rc<ThemeConfig>,
}

impl ResolvedThemes {
    /// 把外观偏好解析为主题三元组。
    ///
    /// 生效规则：用户选择的主题在名字命中且明暗属性与生效模式一致时启用；
    /// 否则回退到该模式的注册表默认主题。选择值不回写，切回匹配的明暗后
    /// 自动恢复用户所选主题。
    fn resolve(appearance: &AppearanceSettings, cx: &App) -> Self {
        let registry = ThemeRegistry::global(cx);
        let eff_mode = effective_mode(appearance.mode, cx);
        let default_for = |mode: ThemeMode| -> Rc<ThemeConfig> {
            if mode.is_dark() {
                registry.default_dark_theme().clone()
            } else {
                registry.default_light_theme().clone()
            }
        };
        let selected = (!appearance.ui_theme.is_empty())
            .then(|| {
                registry
                    .themes()
                    .iter()
                    .find(|(name, _)| name.as_ref() == appearance.ui_theme.as_str())
                    .map(|(_, config)| config.clone())
            })
            .flatten();
        let active = match selected {
            Some(config) if config.mode == eff_mode => config,
            _ => default_for(eff_mode),
        };
        let (light, dark) = if eff_mode.is_dark() {
            (default_for(ThemeMode::Light), active.clone())
        } else {
            (active.clone(), default_for(ThemeMode::Dark))
        };
        Self {
            active,
            light,
            dark,
        }
    }
}

/// 生效的明暗模式：手动档原样返回，跟随系统档读取平台外观。
fn effective_mode(mode: UiThemeMode, cx: &App) -> ThemeMode {
    match mode {
        UiThemeMode::Light => ThemeMode::Light,
        UiThemeMode::Dark => ThemeMode::Dark,
        // Unknown 是反序列化兜底变体，行为与跟随系统一致。
        UiThemeMode::System | UiThemeMode::Unknown => cx.window_appearance().into(),
    }
}

/// 应用界面外观偏好：全局主题、双槽位与密度字号的唯一写路径。
///
/// 开窗前（启动注入）与运行期（设置页切换）都走这里；每次调用都会触发全部
/// 窗口重绘，在途终端会话不受影响。
pub(crate) fn apply_appearance(appearance: &AppearanceSettings, cx: &mut App) {
    let resolved = ResolvedThemes::resolve(appearance, cx);
    let density = appearance.density;
    Theme::update(cx, |theme| {
        theme.apply_config(&resolved.active);
        theme.light_theme = resolved.light;
        theme.dark_theme = resolved.dark;
        theme.font_size = px(density.font_size());
        theme.mono_font_size = px(density.mono_font_size());
    });
}

/// 启动时注册内置主题。
///
/// 注册表对重复名静默跳过（`load_themes_from_str` 的 contains_key 行为），因此
/// 内置主题命名必须避开 gpui-kit 自带的 "Default"；P3 接入用户主题目录时该
/// 规则同样适用。
pub(crate) fn register_builtin(cx: &mut App) {
    if let Err(error) = ThemeRegistry::global_mut(cx).load_themes_from_str(builtin::BUILTIN_THEMES)
    {
        // 内置主题注册失败只损失"出厂深色"这一个选项：注册表仍保有 gpui-kit
        // 默认主题，解析回退路径不受影响，记录后继续启动。
        tracing::error!("注册内置 UI 主题失败，已退回 gpui-kit 默认主题，原因: {error}");
    }
}

/// 系统明暗变化时同步全局主题；仅"跟随系统"档生效，手动选择的明暗不被系统
/// 切换覆盖。`window` 取窗口级外观：Linux 上窗口级上报比应用级可靠（见
/// gpui-kit issue #104，`sync_system_appearance` 的文档注释同此结论）。
pub(crate) fn sync_with_system(window: &mut Window, cx: &mut App) {
    Theme::sync_system_appearance(Some(window), cx);
}
