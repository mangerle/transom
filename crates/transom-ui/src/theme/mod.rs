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
use gpui_kit::{App, Window, px};
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
    /// 明暗不符时先找同族伴侣主题（见 [`companion_name`]），找不到才回退到
    /// 该模式的注册表默认主题。选择值不回写，切回匹配的明暗后自动恢复用户
    /// 所选主题。
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
        let find = |name: &str| -> Option<Rc<ThemeConfig>> {
            registry
                .themes()
                .iter()
                .find(|(registered, _)| registered.as_ref() == name)
                .map(|(_, config)| config.clone())
        };
        let selected = (!appearance.ui_theme.is_empty())
            .then(|| find(&appearance.ui_theme))
            .flatten();
        let active = match selected {
            Some(config) if config.mode == eff_mode => config,
            // 选中的主题明暗不符：优先同族伴侣主题（Islands Dark 切浅色联动
            // Islands Light），让成对主题在明暗切换时保持视觉连续。
            _ => selected
                .as_ref()
                .and_then(|config| companion_name(&config.name, eff_mode))
                .and_then(|name| find(&name))
                .filter(|config| config.mode == eff_mode)
                .unwrap_or_else(|| default_for(eff_mode)),
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

/// 主题族配对约定：名字以 " Dark"/" Light" 结尾的主题视为一族，返回目标明暗
/// 对应的伴侣主题名（如 "JetBrains Islands Dark" 在浅色下联动 "JetBrains
/// Islands Light"）；不带该后缀的名字没有配对语义，返回 None。
///
/// 为什么用命名约定而不是数据字段：主题配置来自上游 `ThemeConfig` 类型，无
/// 法追加族名字段；后缀约定零 schema 成本，且与 Zed 生态的主题命名习惯一致。
fn companion_name(name: &str, mode: ThemeMode) -> Option<String> {
    let family = name
        .strip_suffix(" Dark")
        .or_else(|| name.strip_suffix(" Light"))?;
    let suffix = if mode.is_dark() { " Dark" } else { " Light" };
    Some(format!("{family}{suffix}"))
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

/// 启动时注册全部内置主题（出厂深色 + JetBrains Islands 主题对）。
///
/// 注册表对重复名静默跳过（`load_themes_from_str` 的 contains_key 行为），因此
/// 内置主题命名必须避开 gpui-kit 自带的 "Default"；用户主题目录中的同名文件
/// 同样不会生效（见 [`watch_user_dir`]）。
pub(crate) fn register_builtin(cx: &mut App) {
    for (index, content) in builtin::ALL.iter().enumerate() {
        if let Err(error) = ThemeRegistry::global_mut(cx).load_themes_from_str(content) {
            // 单个内置主题集注册失败只损失对应选项：注册表仍保有 gpui-kit 默认
            // 主题，解析回退路径不受影响，记录后继续启动。
            tracing::error!("注册内置 UI 主题 #{index} 失败，已跳过，原因: {error}");
        }
    }
}

/// 监听用户主题目录（`config_dir/transom/themes/*.json`，gpui-kit ThemeConfig
/// 格式），文件新增或修改时热重载进注册表，设置页即可选用，无需重启。
///
/// 已知局限：注册表对重名静默跳过，用户主题与内置主题同名不会生效，该约束
/// 已写进 `transom_core::paths::themes_dir` 的文档。目录不存在时先创建再监听，
/// 创建失败只损失自定义主题能力，按降级记录、不阻断启动。
pub(crate) fn watch_user_dir(cx: &mut App) {
    let dir = transom_core::paths::themes_dir();
    if let Err(error) = std::fs::create_dir_all(&dir) {
        tracing::warn!(
            "创建用户主题目录失败，自定义主题不可用，目录: {}，原因: {error}",
            dir.display()
        );
        return;
    }
    if let Err(error) = ThemeRegistry::watch_dir(dir, cx, |_| {
        // 注册表变动由 gpui-kit 的 observe_global 自动换装当前槽位并重绘；
        // 新增主题仍需用户在设置页选择后才生效。
        tracing::info!("用户主题目录已重载");
    }) {
        tracing::warn!("监听用户主题目录失败，自定义主题不可用，原因: {error}");
    }
}

/// 系统明暗变化时同步全局主题；仅"跟随系统"档生效，手动选择的明暗不被系统
/// 切换覆盖。`window` 取窗口级外观：Linux 上窗口级上报比应用级可靠（见
/// gpui-kit issue #104，`sync_system_appearance` 的文档注释同此结论）。
pub(crate) fn sync_with_system(window: &mut Window, cx: &mut App) {
    Theme::sync_system_appearance(Some(window), cx);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    /// 解析内置主题 JSON 并做结构断言。
    ///
    /// 为什么断言 JSON 值而不是反序列化成 `ThemeSet`：该契约结构体的 serde
    /// 派生展开过深，在测试编译期会触发 rustc 栈溢出；Value 层断言覆盖结构
    /// 骨架与关键键名，完整 gpui 类型的解析由运行时 `load_themes_from_str`
    /// 的 Result 路径兜底（失败会记录 error 日志并跳过）。
    fn parse(content: &str) -> Value {
        serde_json::from_str(content).expect("内置主题 JSON 解析失败")
    }

    /// 内置主题的结构契约：名字唯一且避开 gpui-kit 内置名、明暗合法、关键
    /// 颜色键与语法高亮节齐全、不携带字号（字号权威在密度档位）。
    #[test]
    fn builtin_themes_follow_contract() {
        let mut names = Vec::new();
        for content in builtin::ALL {
            let doc = parse(content);
            let themes = doc["themes"].as_array().expect("themes 应为数组");
            for theme in themes {
                let name = theme["name"].as_str().expect("name 应为字符串");
                assert!(!name.is_empty());
                assert!(!names.contains(&name.to_string()), "主题名重复: {name}");
                names.push(name.to_string());
                assert!(!name.starts_with("Default"), "与 gpui-kit 内置重名: {name}");
                assert!(
                    theme["mode"] == "dark" || theme["mode"] == "light",
                    "{name} 的 mode 非法"
                );
                for key in ["font.size", "mono_font.size"] {
                    assert!(theme.get(key).is_none(), "{name} 携带了 {key}");
                }
                let colors = &theme["colors"];
                for key in [
                    "background",
                    "foreground",
                    "primary.background",
                    "sidebar.background",
                ] {
                    assert!(colors.get(key).is_some(), "{name} 缺少颜色键 {key}");
                }
                let highlight = &theme["highlight"];
                assert!(
                    highlight.get("editor.background").is_some(),
                    "{name} 缺少编辑器背景"
                );
                assert!(
                    highlight["syntax"]["keyword"]["color"].is_string(),
                    "{name} 缺少语法高亮 keyword"
                );
            }
        }
        assert!(names.contains(&"JetBrains Islands Dark".to_string()));
        assert!(names.contains(&"JetBrains Islands Light".to_string()));
        assert!(names.contains(&"Transom Dark".to_string()));
    }

    /// 主题族配对约定：带 " Dark"/" Light" 后缀的名字能找到伴侣名，其余名字
    /// 没有配对语义。
    #[test]
    fn companion_name_pairs_by_suffix() {
        assert_eq!(
            companion_name("JetBrains Islands Dark", ThemeMode::Light),
            Some("JetBrains Islands Light".into())
        );
        assert_eq!(
            companion_name("JetBrains Islands Light", ThemeMode::Dark),
            Some("JetBrains Islands Dark".into())
        );
        // 同明暗请求返回自身后缀名（是否注册由 resolve 过滤）。
        assert_eq!(
            companion_name("JetBrains Islands Dark", ThemeMode::Dark),
            Some("JetBrains Islands Dark".into())
        );
        assert_eq!(
            companion_name("Transom Dark", ThemeMode::Light),
            Some("Transom Light".into())
        );
        assert_eq!(companion_name("Default", ThemeMode::Dark), None);
    }
}
