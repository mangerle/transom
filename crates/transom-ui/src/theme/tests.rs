//! UI 主题内置契约、中性色阶与静态翻译键单元测试。

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

/// colors 键名必须命中 gpui-kit 的 serde 契约名：`ThemeConfigColors` 没有
/// `deny_unknown_fields`，错名会被静默丢掉，再按模式落到 `ThemeColor::dark()
/// /light()` 兜底，两边都没有的键最终是透明色，表现为"该角色永远不跟随主题"
/// 且没有任何报错。出厂两套主题就栽在 8 个错名上（用的是 `Theme` 的 Rust 字段
/// 名而不是 JSON 键名，如 `drag_border` 应为 `drag.border`、
/// `progress_bar.background` 应为 `progress.bar.background`）。
///
/// 判定方式是把主题反序列化成 `ThemeConfig` 再序列化回来取键名差集：上游增删
/// 键位时测试自动反映，无需手抄 140 个 rename 名。
#[test]
fn builtin_theme_color_keys_are_recognized_by_gpui() {
    for content in builtin::ALL {
        let doc = parse(content);
        for theme in doc["themes"].as_array().expect("themes 应为数组") {
            let config: ThemeConfig = serde_json::from_value(theme.clone())
                .expect("内置主题应能按 gpui-kit 契约反序列化");
            let round_trip = serde_json::to_value(&config).expect("ThemeConfig 应可序列化");
            let source = theme["colors"].as_object().expect("colors 应为对象");
            let known = round_trip["colors"]
                .as_object()
                .expect("回写 colors 应为对象");
            let unknown: Vec<&String> = source
                .keys()
                .filter(|key| !matches!(known.get(*key), Some(value) if !value.is_null()))
                .collect();
            assert!(
                unknown.is_empty(),
                "主题 {} 的 colors 键名不被 gpui-kit 识别: {unknown:?}",
                config.name
            );
        }
    }
}

/// sRGB 的 CIELAB 明度 L*，带 alpha 的十六进制只看前 6 位。
///
/// 为什么用 L* 差而不是 WCAG 对比度比值：比值在近黑与近白两端压缩严重，同一个
/// 比值在浅色区明显、在深色区几乎不可辨——Islands 亮暗两套主题的 border 换算成
/// 比值都接近 1.1，但只有浅色那套看得见。L* 差是跨明暗可比的感知量。
fn l_star(color: &str) -> f64 {
    let digits = color.trim_start_matches('#');
    let body = &digits[..digits.len().min(6)];
    let channel = |offset: usize| -> f64 {
        let raw = u8::from_str_radix(&body[offset..offset + 2], 16).expect("主题色应为十六进制");
        let value = f64::from(raw) / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    let y = 0.2126 * channel(0) + 0.7152 * channel(2) + 0.0722 * channel(4);
    if y > 0.008_856 {
        116.0 * y.cbrt() - 16.0
    } else {
        903.3 * y
    }
}

/// 中性色阶档位契约：结构线必须与它围合的面拉开可辨差，铬件必须与面板底分档。
///
/// 阈值取自 gpui-kit 自带 Default Dark 的层级关系（面板 2.7 / 铬件 7.7 /
/// 结构线 15.2 / 输入框线 19.4，即线比面亮 +12.5），换算成下限后浅色减半——
/// 组件库的语法是"面少线多"，分隔靠线而不是靠加亮的面，内置主题必须供得起
/// 这条线。输入框线单独一档只在深色校验：浅色主题两者同值时仍有 4 档以上的
/// 底色差兜底，强行分档反而要自造上游没有的颜色。
///
/// 只校验内置主题：用户目录里的主题由用户自己负责，越界判定只会把好主题拒掉。
#[test]
fn builtin_neutral_ramp_keeps_separators_visible() {
    let mut violations: Vec<String> = Vec::new();
    for content in builtin::ALL {
        let doc = parse(content);
        for theme in doc["themes"].as_array().expect("themes 应为数组") {
            let name = theme["name"].as_str().unwrap_or_default();
            let dark = theme["mode"] == "dark";
            let colors = &theme["colors"];
            let tone = |key: &str| -> Option<f64> { colors[key].as_str().map(l_star) };
            let (Some(line), Some(panel), Some(raised), Some(input), Some(chrome)) = (
                tone("border"),
                tone("sidebar.background"),
                tone("popover.background"),
                tone("input.border"),
                tone("title_bar.background"),
            ) else {
                violations.push(format!("{name}: 缺少档位校验所需的中性色键"));
                continue;
            };
            let line_gap = if dark { 8.0 } else { 4.0 };
            let chrome_min = if dark { 3.0 } else { 1.5 };
            for (label, gap) in [
                ("结构线 vs 侧栏底", (line - panel).abs()),
                ("结构线 vs 弹窗底", (line - raised).abs()),
            ] {
                if gap < line_gap {
                    violations.push(format!("{name}: {label} ΔL*={gap:.1} 低于 {line_gap:.1}"));
                }
            }
            let input_gap = input - line;
            if dark && input_gap < 3.0 {
                violations.push(format!("{name}: 输入框线未与结构线分档 ΔL*={input_gap:.1}"));
            }
            let chrome_gap = (chrome - panel).abs();
            if !(chrome_min..=9.0).contains(&chrome_gap) {
                violations.push(format!(
                    "{name}: 铬件与面板底差 ΔL*={chrome_gap:.1} 超出 [{chrome_min:.1}, 9.0]"
                ));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "内置主题中性色阶档位不达标：\n{}",
        violations.join("\n")
    );
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
