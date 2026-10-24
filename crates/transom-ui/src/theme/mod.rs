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
use gpui_kit::{App, Global, Window, px};
use std::collections::HashMap;
use std::rc::Rc;
use transom_core::{AppearanceSettings, FOLLOW_UI_THEME, Rgb, TerminalPalette, UiThemeMode};

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
        // 字体族个体偏好覆盖主题自带字体；None 恢复主题值。族变化会触发
        // edit() 内部的系统字体重解析（edit 检测 font_family 变化）。
        if let Some(family) = appearance.ui_font_family.as_deref() {
            theme.font_family = family.into();
        }
    });
}

/// 启动时在内存中注册全部内置主题（出厂深色 + JetBrains Islands 主题对）。
///
/// 这只覆盖"热重载执行前"的窗口：注册表尚未从目录文件重建，首帧就要按
/// `ui_theme` 解析主题，不能依赖异步 reload。目录文件由 [`ensure_builtin_theme_files`]
/// 保证，二者内容一致。
///
/// 注册表对重复名静默跳过（`load_themes_from_str` 的 contains_key 行为），
/// 因此内置主题命名必须避开 gpui-kit 自带的 "Default"。
pub(crate) fn register_builtin(cx: &mut App) {
    for (index, content) in builtin::ALL.iter().enumerate() {
        if let Err(error) = ThemeRegistry::global_mut(cx).load_themes_from_str(content) {
            // 单个内置主题集注册失败只损失对应选项：注册表仍保有 gpui-kit 默认
            // 主题，解析回退路径不受影响，记录后继续启动。
            tracing::error!("注册内置 UI 主题 #{index} 失败，已跳过，原因: {error}");
        }
    }
}

/// 从主题 JSON 提取伴生的终端调色板（自定义 `terminal` 节）。
///
/// gpui-kit 的 `ThemeSet` 契约没有终端概念，未知字段会被 serde 静默忽略，
/// 因此终端配色作为旁路字段与 UI 主题共存于同一文件；解析层自行读取，
/// 键缺失或字段非法时返回空——主题可以不带终端配色。
fn extract_terminal_palettes(content: &str) -> Vec<(String, TerminalPalette)> {
    let Ok(doc) = serde_json::from_str::<serde_json::Value>(content) else {
        return Vec::new();
    };
    let Some(themes) = doc["themes"].as_array() else {
        return Vec::new();
    };
    let parse_rgb =
        |value: &serde_json::Value| -> Option<Rgb> { value.as_str().and_then(Rgb::parse_hex) };
    themes
        .iter()
        .filter_map(|theme| {
            let name = theme["name"].as_str()?.to_string();
            let terminal = &theme["terminal"];
            let background = parse_rgb(&terminal["background"])?;
            let foreground = parse_rgb(&terminal["foreground"])?;
            let cursor = parse_rgb(&terminal["cursor"])?;
            let selection = parse_rgb(&terminal["selection"])?;
            let ansi_values = terminal["ansi"].as_array()?;
            if ansi_values.len() != 16 {
                return None;
            }
            let mut ansi = [Rgb::new(0, 0, 0); 16];
            for (slot, value) in ansi_values.iter().enumerate() {
                ansi[slot] = parse_rgb(value)?;
            }
            Some((
                name,
                TerminalPalette {
                    background,
                    foreground,
                    cursor,
                    selection,
                    ansi,
                },
            ))
        })
        .collect()
}

/// 主题名到伴生终端调色板的绑定表：启动与热重载时从内置与用户目录主题
/// 重建。热重载后新增的伴生主题要重启才会进入绑定表（目录变动只触发
/// 注册表 reload，无回调），属已知局限。
pub(crate) fn rebuild_terminal_bindings(cx: &mut App) {
    let mut bindings: HashMap<String, TerminalPalette> = HashMap::new();
    for content in builtin::ALL {
        for (name, palette) in extract_terminal_palettes(content) {
            bindings.insert(name, palette);
        }
    }
    let dir = transom_core::paths::themes_dir();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("json")
                && let Ok(content) = std::fs::read_to_string(&path)
            {
                for (name, palette) in extract_terminal_palettes(&content) {
                    bindings.insert(name, palette);
                }
            }
        }
    }
    cx.set_global(TerminalPaletteBindings(bindings));
}

/// 主题名 → 伴生终端调色板。
#[derive(Default)]
struct TerminalPaletteBindings(HashMap<String, TerminalPalette>);

impl Global for TerminalPaletteBindings {}

/// 终端配色的最终裁决：显式预设（`theme_name` 为具体名字且 palette 已具体化）
/// 直接沿用；跟随模式（哨兵值）解析当前 UI 主题的伴生调色板，主题未携带时
/// 回退内置默认（One Dark）。
pub(crate) fn resolve_terminal_palette(
    profile_theme_name: &str,
    active_ui_theme_name: &str,
    cx: &App,
) -> Option<TerminalPalette> {
    if profile_theme_name != FOLLOW_UI_THEME {
        return None;
    }
    let bindings = cx.try_global::<TerminalPaletteBindings>()?;
    bindings
        .0
        .get(active_ui_theme_name)
        .cloned()
        .or_else(|| Some(TerminalPalette::default()))
}

/// 把内置主题落盘为用户主题目录内的受管文件。
///
/// 为什么文件化：主题注册表的热重载以"目录全量重建"为语义——`reload` 会
/// 清空注册表、只装回 gpui-kit 内置 Default 与目录文件。纯内存注册的内置
/// 主题在目录任意变动后都会被清掉（P3 首版即栽在这里：界面只剩 Default）。
/// 落盘后内置主题与用户主题在同一套 reload 语义下共存，且用户可以直接复制
/// 内置文件改造成自己的主题。
///
/// 受管判定：文件顶层 `author == "transom"`。存在且匹配则随版本覆盖更新；
/// 不匹配（用户改过或换成自己的内容）视为用户主题，永不覆盖。
fn ensure_builtin_theme_files(dir: &std::path::Path) {
    for (file_name, content) in builtin::MANAGED_FILES {
        let path = dir.join(file_name);
        if let Ok(existing) = std::fs::read_to_string(&path)
            && let Ok(doc) = serde_json::from_str::<serde_json::Value>(&existing)
            && doc.get("author").and_then(|author| author.as_str()) != Some("transom")
        {
            // author 已被用户改动：这是用户自己的主题了，跳过覆盖。
            continue;
        }
        if let Err(error) = std::fs::write(&path, content) {
            tracing::warn!(
                "写入内置主题文件失败，热重载后该主题将不可见，文件: {}，原因: {error}",
                path.display()
            );
        }
    }
}

/// 初始化并监听用户主题目录（`config_dir/transom/themes/*.json`，gpui-kit
/// ThemeConfig 格式），文件新增或修改时热重载进注册表，设置页即可选用，
/// 无需重启。
///
/// 三步各有职责：落盘受管内置文件（reload 语义需要）、内存注册内置主题
/// （reload 异步执行前首帧就要能解析到目标主题）、目录监听。目录创建失败
/// 只损失自定义主题能力，按降级记录、不阻断启动。
pub(crate) fn watch_user_dir(cx: &mut App) {
    let dir = transom_core::paths::themes_dir();
    if let Err(error) = std::fs::create_dir_all(&dir) {
        tracing::warn!(
            "创建用户主题目录失败，自定义主题不可用，目录: {}，原因: {error}",
            dir.display()
        );
        return;
    }
    ensure_builtin_theme_files(&dir);
    register_builtin(cx);
    rebuild_terminal_bindings(cx);
    if let Err(error) = ThemeRegistry::watch_dir(dir, cx, |cx| {
        // 注册表变动由 gpui-kit 的 observe_global 自动换装当前槽位并重绘；
        // 新增主题仍需用户在设置页选择后才生效。伴生终端调色板一并重建
        // （仅启动与本次回调；后续文件变动需重启刷新绑定，见绑定表文档）。
        rebuild_terminal_bindings(cx);
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
    use gpui_kit::component::theme::ThemeSet;
    use regex::Regex;
    use serde_json::Value;
    use std::sync::LazyLock;

    /// 静态 t! 键提取：取宏调用里的引号字面量作为键。
    static KEY_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"\bt!\("([^"]+)"\)"#).expect("正则合法"));

    /// 收集 transom-ui 全部 Rust 源码（含子模块）。
    fn source_files() -> Vec<(String, String)> {
        let mut files = Vec::new();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        for entry in walkdir_like(&root) {
            if entry.extension().and_then(|e| e.to_str()) == Some("rs")
                && let Ok(content) = std::fs::read_to_string(&entry)
            {
                files.push((entry.display().to_string(), content));
            }
        }
        files
    }

    /// 简易目录递归（避免为此引入 walkdir 依赖）。
    fn walkdir_like(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
        let mut out = Vec::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(current) = stack.pop() {
            if let Ok(entries) = std::fs::read_dir(&current) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        stack.push(path);
                    } else {
                        out.push(path);
                    }
                }
            }
        }
        out
    }

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

    /// 内置主题必须通过 gpui-kit `ThemeSet` 的真实反序列化契约：Value 层断言
    /// 挡不住键值格式错误（如 font_style 的枚举写法），而注册失败只留一条
    /// error 日志、界面上主题直接消失，必须在这里挡住。
    #[test]
    fn builtin_themes_deserialize_with_gpui_contract() {
        for content in builtin::ALL {
            let result = serde_json::from_str::<ThemeSet>(content);
            assert!(
                result.is_ok(),
                "内置主题不符合 gpui-kit ThemeSet 契约: {:?}",
                result.err()
            );
        }
    }

    /// 注册逻辑本体：不依赖 App，直接验证 load_themes_from_str 能把内置主题
    /// 装进注册表并被 sorted_themes 列出（复现真实启动序列的核心步骤）。
    #[test]
    fn builtin_themes_load_into_registry() {
        let mut registry = ThemeRegistry::default();
        for content in builtin::ALL {
            registry
                .load_themes_from_str(content)
                .expect("内置主题注册失败");
        }
        let names: Vec<String> = registry
            .sorted_themes()
            .into_iter()
            .map(|config| config.name.to_string())
            .collect();
        for expected in [
            "Transom Dark",
            "JetBrains Islands Dark",
            "JetBrains Islands Light",
        ] {
            assert!(
                names.iter().any(|name| name == expected),
                "注册表中缺少 {expected}，实际: {names:?}"
            );
        }
    }

    /// 受管内置文件三态：不存在则写入、author 是 transom 则随版本覆盖、
    /// author 被用户改过则永不覆盖。
    #[test]
    fn managed_theme_files_respect_user_ownership() {
        let dir = tempfile::tempdir().expect("创建临时目录失败");
        let dir = dir.path();

        ensure_builtin_theme_files(dir);
        let dark = dir.join("transom-dark.json");
        assert!(dark.exists(), "首次应写入受管文件");

        // 用户改过 author：视为用户主题，覆盖被跳过。
        std::fs::write(
            &dark,
            r#"{"name":"我的主题","author":"someone","themes":[]}"#,
        )
        .unwrap();
        ensure_builtin_theme_files(dir);
        let doc: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&dark).unwrap())
            .expect("用户文件应保持 JSON 合法");
        assert_eq!(doc["name"], "我的主题", "author 被改过的文件不应被覆盖");

        // author 保持 transom：随版本覆盖更新。
        std::fs::write(&dark, r#"{"name":"旧","author":"transom","themes":[]}"#).unwrap();
        ensure_builtin_theme_files(dir);
        let doc: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&dark).unwrap())
            .expect("覆盖后的文件应保持 JSON 合法");
        assert_eq!(
            doc["themes"][0]["name"], "Transom Dark",
            "受管文件应随版本覆盖"
        );
    }

    /// 静态 i18n 键完整性：扫描 transom-ui 源码里全部 t! 宏调用的字面量键，
    /// 断言每种语言都有定义。`t!` 缺键时静默返回键名本身（界面上直接显示
    /// "settings.xxx"），编译器拦不住，只能在这里挡。
    #[test]
    fn all_static_translation_keys_are_defined() {
        let mut missing: Vec<String> = Vec::new();
        for locale in ["zh-CN", "en"] {
            for (path, content) in source_files() {
                for caps in KEY_RE.captures_iter(&content) {
                    let key = caps.get(1).unwrap().as_str();
                    // rust_i18n 的缺键行为是返回键名字符串，借此判定缺失。
                    let translated = rust_i18n::t!(key, locale = locale).to_string();
                    if translated == key {
                        missing.push(format!("{locale}: {key} ({path})"));
                    }
                }
            }
        }
        assert!(missing.is_empty(), "缺失翻译键: {missing:#?}");
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
