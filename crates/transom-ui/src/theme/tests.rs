//! UI 主题内置契约、中性色阶与静态翻译键单元测试。

use super::*;
use gpui_kit::component::theme::ThemeSet;
use regex::Regex;
use serde_json::Value;
use std::sync::LazyLock;

/// 静态 t! 键提取：取宏调用里的引号字面量作为键（含带命名参数的调用）。
static KEY_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"\bt!\("([^"]+)""#).expect("正则合法"));

/// 收集工作区全部 Rust 源码：翻译键分布在 ui 与 terminal 等 crate，必须一起校验。
fn workspace_source_files() -> Vec<(String, String)> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    for base in ["crates", "apps"] {
        for entry in walkdir_like(&root.join(base)) {
            let path = entry.display().to_string();
            if path.contains("/target/") {
                continue;
            }
            if entry.extension().and_then(|e| e.to_str()) == Some("rs")
                && let Ok(content) = std::fs::read_to_string(&entry)
            {
                files.push((path, content));
            }
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
        for (path, content) in workspace_source_files() {
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

// ---------------------------------------------------------------------------
// 国际化守卫：UI 源码不得再出现硬编码中文文案
// ---------------------------------------------------------------------------

/// 不面向界面的调用：日志、断言、内部诊断。这些位置的字符串先整体剔除，
/// 避免用逐条白名单维护；剔除后仍出现中文，才视为界面文案漏网。
const SILENT_CALLS: &[&str] = &[
    "log::trace!(",
    "log::debug!(",
    "log::info!(",
    "log::warn!(",
    "log::error!(",
    "tracing::trace!(",
    "tracing::debug!(",
    "tracing::info!(",
    "tracing::warn!(",
    "tracing::error!(",
    "trace!(",
    "debug!(",
    "info!(",
    "warn!(",
    "error!(",
    "println!(",
    "eprintln!(",
    "panic!(",
    "debug_assert!(",
    "debug_assert_eq!(",
    "debug_assert_ne!(",
    "assert!(",
    "assert_eq!(",
    "assert_ne!(",
    "AppError::with_detail(",
    "io::Error::new(",
    "expect(",
    "#[error(",
];

/// 参与守卫扫描的界面源码：主界面与终端渲染层。
fn ui_source_files() -> Vec<(String, String)> {
    let mut out = Vec::new();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for base in ["crates/transom-ui/src", "crates/transom-terminal/src"] {
        for entry in walkdir_like(&root.join(base)) {
            let path = entry.display().to_string();
            if path.ends_with("tests.rs") {
                continue;
            }
            if entry.extension().and_then(|e| e.to_str()) == Some("rs")
                && let Ok(content) = std::fs::read_to_string(&entry)
            {
                out.push((path, content));
            }
        }
    }
    out
}

#[test]
fn ui_sources_have_no_hardcoded_chinese() {
    let mut offenders = Vec::new();
    for (path, content) in ui_source_files() {
        for (index, literal) in chinese_literals(&content) {
            if allowlisted_literal(&path, &literal) {
                continue;
            }
            offenders.push(format!("{path}:{index}: {literal}"));
        }
    }
    assert!(
        offenders.is_empty(),
        "UI 源码仍有硬编码中文文案，请改用 rust_i18n::t!()：\n{}",
        offenders.join("\n")
    );
}

/// 有意保留的中文：内部比较标记、内部诊断与语言名，不参与界面语言切换。
///
/// 这几条都不是界面文案：`本地/远端` 是属性弹窗的来源判别值，`校验` 用于识别
/// 校验失败以强制重传，`连接已断开` 与 `权限` 参与错误分支判定（诊断详情只在
/// 中文界面展示），`上报 shell` 只拼进日志；语言名按自身语言书写。
fn allowlisted_literal(path: &str, literal: &str) -> bool {
    let literal = literal.trim();
    (literal == "本地"
        && (path.ends_with("sftp/properties.rs")
            || path.ends_with("sftp/modals.rs")
            || path.ends_with("sftp/modals/properties.rs")))
        || (literal == "远端" && path.ends_with("sftp_handlers.rs"))
        || (literal == "中文" && path.ends_with("settings/general_page.rs"))
        || (literal == "校验"
            && (path.ends_with("sftp/queue_actions/mod.rs")
                || path.ends_with("sftp/queue_actions/task_control.rs")))
        || (literal == "连接已断开" && path.ends_with("connection/mod.rs"))
        || (literal == "权限" && path.ends_with("editor/banner.rs"))
        || (literal == "，上报 shell: {shell}" && path.ends_with("shell_integration.rs"))
}

/// 剔除测试模块、注释与静默调用后，返回剩余的中文字面量（行号从 1 起）。
fn chinese_literals(src: &str) -> Vec<(usize, String)> {
    let cut = src.find("#[cfg(test)]").unwrap_or(src.len());
    let code = strip_silent_regions(&src[..cut]);
    let mut out = Vec::new();
    for (index, line) in code.lines().enumerate() {
        for literal in string_literals(line) {
            if literal.chars().any(is_cjk) {
                out.push((index + 1, literal));
            }
        }
    }
    out
}

fn is_cjk(c: char) -> bool {
    ('\u{4e00}'..='\u{9fff}').contains(&c)
}

/// 把注释与静默调用替换为空格，保留换行以维持行号。
fn strip_silent_regions(src: &str) -> String {
    let bytes = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'/') {
            let end = src[i..].find('\n').map_or(bytes.len(), |offset| i + offset);
            blank_range(&src[i..end], &mut out);
            i = end;
            continue;
        }
        if bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'*') {
            let end = skip_block_comment(bytes, i);
            blank_range(&src[i..end], &mut out);
            i = end;
            continue;
        }
        if !inside_ident(bytes, i)
            && let Some(name) = SILENT_CALLS
                .iter()
                .find(|name| src[i..].starts_with(**name))
        {
            let open = i + name.len() - 1;
            let end = skip_balanced(bytes, open);
            blank_range(&src[i..end], &mut out);
            i = end;
            continue;
        }
        if bytes[i] == b'"' {
            let end = skip_string(bytes, i);
            out.push_str(&src[i..end]);
            i = end;
            continue;
        }
        if bytes[i] == b'\'' && looks_like_char(bytes, i) {
            let end = skip_char(bytes, i);
            out.push_str(&src[i..end]);
            i = end;
            continue;
        }
        let len = src[i..].chars().next().map_or(1, char::len_utf8);
        out.push_str(&src[i..i + len]);
        i += len;
    }
    out
}

fn inside_ident(bytes: &[u8], index: usize) -> bool {
    index > 0 && (bytes[index - 1].is_ascii_alphanumeric() || bytes[index - 1] == b'_')
}

fn blank_range(range: &str, out: &mut String) {
    for c in range.chars() {
        out.push(if c == '\n' { '\n' } else { ' ' });
    }
}

fn skip_string(bytes: &[u8], start: usize) -> usize {
    let mut i = start + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'"' => return i + 1,
            _ => i += 1,
        }
    }
    bytes.len()
}

fn looks_like_char(bytes: &[u8], start: usize) -> bool {
    match bytes.get(start + 1) {
        Some(b'\\') => bytes.get(start + 3) == Some(&b'\''),
        Some(_) => bytes.get(start + 2) == Some(&b'\''),
        None => false,
    }
}

fn skip_char(bytes: &[u8], start: usize) -> usize {
    if bytes.get(start + 1) == Some(&b'\\') {
        (start + 4).min(bytes.len())
    } else {
        (start + 3).min(bytes.len())
    }
}

fn skip_block_comment(bytes: &[u8], start: usize) -> usize {
    let mut depth = 1;
    let mut i = start + 2;
    while i < bytes.len() {
        if bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'*') {
            depth += 1;
            i += 2;
        } else if bytes[i] == b'*' && bytes.get(i + 1) == Some(&b'/') {
            depth -= 1;
            i += 2;
            if depth == 0 {
                return i;
            }
        } else {
            i += 1;
        }
    }
    bytes.len()
}

/// 从 `(` 开始跳到配对的 `)` 之后；字符串、字符与注释里的括号不计入深度。
fn skip_balanced(bytes: &[u8], open: usize) -> usize {
    let mut depth = 0usize;
    let mut i = open;
    while i < bytes.len() {
        if bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'/') {
            i = bytes[i..]
                .iter()
                .position(|&b| b == b'\n')
                .map_or(bytes.len(), |offset| i + offset);
            continue;
        }
        if bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'*') {
            i = skip_block_comment(bytes, i);
            continue;
        }
        if bytes[i] == b'"' {
            i = skip_string(bytes, i);
            continue;
        }
        if bytes[i] == b'\'' && looks_like_char(bytes, i) {
            i = skip_char(bytes, i);
            continue;
        }
        match bytes[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return i + 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    bytes.len()
}

/// 单行内的双引号字符串字面量；转义引号不计为结束。
fn string_literals(line: &str) -> Vec<String> {
    let bytes = line.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'"' {
            i += 1;
            continue;
        }
        let mut value = String::new();
        i += 1;
        while i < bytes.len() {
            match bytes[i] {
                b'\\' => {
                    if let Some(&next) = bytes.get(i + 1) {
                        value.push(next as char);
                    }
                    i += 2;
                }
                b'"' => {
                    i += 1;
                    break;
                }
                _ => {
                    let len = line[i..].chars().next().map_or(1, char::len_utf8);
                    value.push_str(&line[i..i + len]);
                    i += len;
                }
            }
        }
        out.push(value);
    }
    out
}

/// 组件层必须走设计令牌：字号、间距、控件尺寸与弹窗宽度不能在这里重新写
/// `text_xs()` / `px(...)`，否则页面迁移到组件层后仍会随密度切换失调。
#[test]
fn widgets_use_design_tokens() {
    let mut offenders = Vec::new();
    for (path, content) in ui_source_files() {
        if !path.contains("/widgets/") {
            continue;
        }
        for (index, line) in content.lines().enumerate() {
            let magic_px = line
                .match_indices("px(")
                .any(|(index, _)| !line[index + 3..].starts_with("rems("));
            if line.contains("text_xs(")
                || line.contains("text_sm(")
                || magic_px
                || line.contains("gap_1(")
                || line.contains("gap_2(")
                || line.contains("gap_3(")
                || line.contains("gap_4(")
            {
                offenders.push(format!("{path}:{}: {}", index + 1, line.trim()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "widgets 组件必须使用 theme::tokens 中的字号/间距/控件令牌：\n{}",
        offenders.join("\n")
    );
}

/// 菜单项必须走 widgets 构造器：`PopupMenuItem::new(...)` 紧随 `.icon(...)`
/// 是迁移前的写法，散落回页面会让图标顺序与禁用语义各自漂移。
///
/// 例外只保留两处真实差异：带真实 `Action` 的条目（动作自带键位展示）与下拉
/// 里的 `.checked(...)` 选中项，它们不属于同一语义。
#[test]
fn menu_items_use_widget_constructor() {
    let mut offenders = Vec::new();
    for (path, content) in ui_source_files() {
        // 只守卫 transom-ui：transom-terminal 不能反向依赖 UI 组件层。
        if path.contains("/widgets/") || !path.contains("/transom-ui/src/") {
            continue;
        }
        let lines: Vec<&str> = content.lines().collect();
        for (index, line) in lines.iter().enumerate() {
            if !line.contains("PopupMenuItem::new(") && !line.contains("PopupMenuItem::new()") {
                continue;
            }
            let window = lines
                .iter()
                .skip(index)
                .take(3)
                .any(|next| next.trim_start().starts_with(".icon("));
            if window {
                offenders.push(format!("{}:{}: {}", path, index + 1, line.trim()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "菜单项请使用 crate::widgets::menu_item / MenuItem：\n{}",
        offenders.join("\n")
    );
}

/// 外壳只能持有状态容器一个字段：`AppShell` 再平铺回字段就意味着功能模块又能
/// 通过兄弟路径直接读写全局状态，封装会重新退化。
#[test]
fn app_shell_keeps_single_state_field() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/app/mod.rs");
    let content = std::fs::read_to_string(&path).expect("读取 app/mod.rs 失败");
    let start = content
        .find("pub(crate) struct AppShell {")
        .expect("未找到 AppShell 定义");
    let end = content[start..].find("\n}\n").expect("AppShell 定义未闭合") + start;
    let body = &content[start..end];
    let fields: Vec<&str> = body
        .lines()
        .skip(1)
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("//") && line.ends_with(','))
        .collect();
    assert_eq!(
        fields,
        vec!["state: AppState,"],
        "AppShell 只允许持有 state 一个字段，新增状态请放进 AppState 或 feature 结构体"
    );
}

/// 监控面板的渲染与面板状态必须归属 `MonitorView` 实体：`monitor/` 下只允许
/// `ops.rs` 保留 `impl AppShell`（采样调度、任务生命周期与跨域动作回交外壳）。
/// 一旦渲染方法重新散落到外壳，说明实体化被回退了。
#[test]
fn monitor_rendering_belongs_to_entity() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/monitor");
    let mut offenders = Vec::new();
    for (path, content) in walkdir_like(&root)
        .into_iter()
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("rs"))
        .filter_map(|path| {
            let content = std::fs::read_to_string(&path).ok()?;
            Some((path.display().to_string(), content))
        })
    {
        if path.ends_with("ops.rs") {
            continue;
        }
        if content.contains("impl AppShell") {
            offenders.push(path);
        }
    }
    assert!(
        offenders.is_empty(),
        "监控渲染必须以 impl MonitorView 实现，不得挂在 AppShell 上：\n{}",
        offenders.join("\n")
    );
}

/// 编辑面板实体回调回交外壳时，被调用方法不得再读写编辑面板实体。
///
/// 与监控面板同一约束：实体点击/输入回调执行期间 `EditorPanels` 已被租借，
/// 外壳方法里再 `state.editors.read/update` 会触发 “cannot update while it is
/// already being updated” 崩溃。这里扫描编辑器目录里的 `with_shell` 目标，
/// 在编辑器、对比面板与 SFTP 动作源码中定位其函数体做检查。
#[test]
fn editor_with_shell_targets_do_not_reenter_panel() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut targets: Vec<String> = Vec::new();
    for entry in walkdir_like(&root.join("editor")) {
        if entry.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&entry) else {
            continue;
        };
        collect_with_shell_targets(&content, &mut targets);
    }
    assert!(!targets.is_empty(), "未找到任何 with_shell 调用，守卫失效");

    let mut sources = Vec::new();
    for entry in walkdir_like(&root.join("editor")) {
        if entry.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        if let Ok(content) = std::fs::read_to_string(&entry) {
            sources.push(content);
        }
    }
    for extra in ["diff_viewer/mod.rs", "sftp/nav_actions/init.rs"] {
        if let Ok(content) = std::fs::read_to_string(root.join(extra)) {
            sources.push(content);
        }
    }

    let mut offenders = Vec::new();
    let mut checked = 0usize;
    for name in targets {
        for source in &sources {
            let Some(body) = function_body(source, &name) else {
                continue;
            };
            checked += 1;
            if body.contains("state.editors") {
                offenders.push(name.clone());
            }
        }
    }
    assert!(checked > 0, "未能定位任何 with_shell 目标函数体，守卫失效");
    offenders.sort();
    offenders.dedup();
    assert!(
        offenders.is_empty(),
        "以下外壳方法在编辑面板回调里被调用，却再次访问 EditorPanels，会触发双重租借：\n{}",
        offenders.join("\n")
    );
}

/// 收集 `with_shell(cx, |shell, ..| shell.foo(..))` 的目标方法名。
fn collect_with_shell_targets(content: &str, out: &mut Vec<String>) {
    for capture in content.split("with_shell(cx,").skip(1) {
        let tail: String = capture.chars().take(400).collect();
        let Some(start) = tail.find("shell.") else {
            continue;
        };
        let after = &tail[start + "shell.".len()..];
        let name: String = after
            .chars()
            .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
            .collect();
        if !name.is_empty() {
            out.push(name);
        }
    }
}

/// 粗粒度提取 `fn name(` 的函数体：按花括号配平，足以覆盖本仓库风格。
fn function_body<'a>(source: &'a str, name: &str) -> Option<&'a str> {
    let pos = source.find(&format!("fn {name}("))?;
    let open = source[pos..].find('{').map(|i| pos + i)?;
    let mut depth = 0usize;
    for (offset, ch) in source[open..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&source[open..=open + offset]);
                }
            }
            _ => {}
        }
    }
    None
}

/// 外壳不得通过 `Deref` 继承状态方法：那会让 `AppState` 的方法隐式长在
/// `AppShell` 上，调用点看不出归属，且任意模块都能拿到 `&mut AppState`
/// 改状态而不触发重绘。状态访问必须显式写出 `.state`。
#[test]
fn app_shell_does_not_deref_state() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut offenders = Vec::new();
    for entry in walkdir_like(&root) {
        if entry.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&entry) else {
            continue;
        };
        if entry.ends_with("theme/tests.rs") {
            continue;
        }
        if content.contains("ops::Deref for AppShell")
            || content.contains("ops::DerefMut for AppShell")
            || content.contains("impl Deref for AppShell")
            || content.contains("impl DerefMut for AppShell")
        {
            offenders.push(entry.display().to_string());
        }
    }
    assert!(
        offenders.is_empty(),
        "禁止为 AppShell 实现 Deref/DerefMut，请显式写 self.state.xxx：\n{}",
        offenders.join("\n")
    );
}

/// 监控面板也不得再有 `with_shell` 同步回交通道。
#[test]
fn monitor_panel_has_no_sync_callback_channel() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/monitor");
    let mut offenders = Vec::new();
    for entry in walkdir_like(&root) {
        if entry.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&entry) else {
            continue;
        };
        if content.contains("with_shell(") {
            offenders.push(entry.display().to_string());
        }
    }
    assert!(
        offenders.is_empty(),
        "监控面板不得再使用 with_shell 同步回交外壳：\n{}",
        offenders.join("\n")
    );
}
