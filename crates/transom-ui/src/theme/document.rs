//! 主题文档解析、校验与序列化。
//!
//! gpui-kit 的 `ThemeSet` 不认识终端配色，因此终端色板作为同一主题对象里的
//! 旁路 `terminal` 节保存。本模块负责把这份外部 JSON 解析成强类型配置，并把
//! 用户编辑结果重新序列化成稳定文档；UI 渲染与注册表热重载共用这套逻辑。

use gpui_kit::component::theme::ThemeConfig;
use gpui_kit::component::try_parse_color;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::hash::{Hash, Hasher};
use transom_core::{Rgb, TerminalPalette};

/// 外部主题文件上限。
///
/// 主题是纯文本小文件，1 MiB 已足以覆盖完整调色板和说明字段；不设上限会让
/// 误选的大文件在 UI 线程解析时造成不可控内存与延迟。
pub(crate) const MAX_THEME_DOCUMENT_BYTES: usize = 1024 * 1024;

/// 主题文档解析错误。
#[derive(Debug, thiserror::Error)]
pub(crate) enum ThemeDocumentError {
    #[error("主题文件超过 1 MiB 上限，实际大小: {actual} 字节")]
    TooLarge { actual: usize },
    #[error("主题 JSON 无法解析，原因: {source}")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("主题文件缺少 themes 数组")]
    MissingThemes,
    #[error("主题编辑器一次只能保存一个主题，当前文件包含 {count} 个主题")]
    MultipleThemes { count: usize },
    #[error("主题名称不能为空")]
    EmptyName,
    #[error("主题颜色字段 {field} 的值无效: {value}")]
    InvalidUiColor { field: String, value: String },
    #[error("终端颜色字段 {field} 的值无效: {value}")]
    InvalidTerminalColor { field: String, value: String },
    #[error("终端 ANSI 配色必须恰好包含 16 个颜色，当前为 {count} 个")]
    InvalidAnsiCount { count: usize },
    #[error("主题文档无法序列化，原因: {source}")]
    Serialize {
        #[source]
        source: serde_json::Error,
    },
    #[error("主题注册失败，原因: {message}")]
    Registry { message: String },
}

/// 编辑器与预览共用的单主题解析结果。
#[derive(Debug, Clone)]
pub(crate) struct ThemeDraft {
    pub(crate) set_name: String,
    pub(crate) config: ThemeConfig,
    pub(crate) terminal_palette: Option<TerminalPalette>,
}

/// 解析单主题文档；编辑器、预览与保存都走这一条路径。
pub(crate) fn parse_single_theme_document(content: &str) -> Result<ThemeDraft, ThemeDocumentError> {
    if content.len() > MAX_THEME_DOCUMENT_BYTES {
        return Err(ThemeDocumentError::TooLarge {
            actual: content.len(),
        });
    }
    let root: Value =
        serde_json::from_str(content).map_err(|source| ThemeDocumentError::Json { source })?;
    let themes = root
        .get("themes")
        .and_then(Value::as_array)
        .ok_or(ThemeDocumentError::MissingThemes)?;
    if themes.len() != 1 {
        return Err(ThemeDocumentError::MultipleThemes {
            count: themes.len(),
        });
    }
    let theme_value = themes
        .first()
        .cloned()
        .ok_or(ThemeDocumentError::MissingThemes)?;
    let config: ThemeConfig = serde_json::from_value(theme_value.clone())
        .map_err(|source| ThemeDocumentError::Json { source })?;
    if config.name.trim().is_empty() {
        return Err(ThemeDocumentError::EmptyName);
    }
    validate_primary_ui_colors(&config)?;
    let terminal_palette = terminal_palette_from_theme_value(&theme_value)?;
    Ok(ThemeDraft {
        set_name: root
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("user-theme")
            .to_string(),
        config,
        terminal_palette,
    })
}

fn validate_primary_ui_colors(config: &ThemeConfig) -> Result<(), ThemeDocumentError> {
    for (field, value) in [
        ("background", config.colors.background.as_deref()),
        ("foreground", config.colors.foreground.as_deref()),
        ("border", config.colors.border.as_deref()),
        ("primary.background", config.colors.primary.as_deref()),
        ("accent.background", config.colors.accent.as_deref()),
    ] {
        let Some(value) = value else {
            continue;
        };
        if try_parse_color(value).is_err() {
            return Err(ThemeDocumentError::InvalidUiColor {
                field: field.to_string(),
                value: value.to_string(),
            });
        }
    }
    Ok(())
}

/// 从任意主题文件中提取有效的伴生终端调色板。
///
/// 热重载采用“尽量加载”语义：单个主题缺少或写错终端节时只损失该主题的跟随
/// 终端色，不影响 UI 主题本身注册。编辑器保存则使用严格解析，避免把错误静默
/// 写入用户目录。
pub(crate) fn extract_terminal_palettes(content: &str) -> Vec<(String, TerminalPalette)> {
    let Ok(root) = serde_json::from_str::<Value>(content) else {
        return Vec::new();
    };
    let Some(themes) = root.get("themes").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut palettes = Vec::with_capacity(themes.len());
    for theme in themes {
        let Some(name) = theme.get("name").and_then(Value::as_str) else {
            continue;
        };
        let Ok(Some(palette)) = terminal_palette_from_theme_value(theme) else {
            continue;
        };
        palettes.push((name.to_string(), palette));
    }
    palettes
}

fn terminal_palette_from_theme_value(
    theme: &Value,
) -> Result<Option<TerminalPalette>, ThemeDocumentError> {
    let Some(terminal) = theme.get("terminal") else {
        return Ok(None);
    };
    if terminal.is_null() {
        return Ok(None);
    }
    let parse = |field: &str| -> Result<Rgb, ThemeDocumentError> {
        let value = terminal.get(field).and_then(Value::as_str).ok_or_else(|| {
            ThemeDocumentError::InvalidTerminalColor {
                field: field.to_string(),
                value: "<missing>".to_string(),
            }
        })?;
        parse_terminal_color(field, value)
    };
    let parse_optional = |field: &str, fallback: Rgb| -> Result<Rgb, ThemeDocumentError> {
        match terminal.get(field).and_then(Value::as_str) {
            Some(value) => parse_terminal_color(field, value),
            None => Ok(fallback),
        }
    };
    let ansi_values = terminal
        .get("ansi")
        .and_then(Value::as_array)
        .ok_or_else(|| ThemeDocumentError::InvalidTerminalColor {
            field: "ansi".to_string(),
            value: "<missing>".to_string(),
        })?;
    if ansi_values.len() != 16 {
        return Err(ThemeDocumentError::InvalidAnsiCount {
            count: ansi_values.len(),
        });
    }
    let mut ansi = [Rgb::new(0, 0, 0); 16];
    for (slot, value) in ansi_values.iter().enumerate() {
        let value = value
            .as_str()
            .ok_or_else(|| ThemeDocumentError::InvalidTerminalColor {
                field: format!("ansi[{slot}]"),
                value: "<non-string>".to_string(),
            })?;
        ansi[slot] = parse_terminal_color(&format!("ansi[{slot}]"), value)?;
    }
    let foreground_intense = terminal
        .get("foregroundIntense")
        .and_then(Value::as_str)
        .map(|value| parse_terminal_color("foregroundIntense", value))
        .transpose()?;
    let defaults = TerminalPalette::default();
    Ok(Some(TerminalPalette {
        background: parse("background")?,
        foreground: parse("foreground")?,
        foreground_intense,
        cursor: parse("cursor")?,
        selection: parse("selection")?,
        line_highlight: parse_optional("lineHighlight", defaults.line_highlight)?,
        find_match_background: parse_optional(
            "findMatchBackground",
            defaults.find_match_background,
        )?,
        find_match_border: parse_optional("findMatchBorder", defaults.find_match_border)?,
        ansi,
    }))
}

fn parse_terminal_color(field: &str, value: &str) -> Result<Rgb, ThemeDocumentError> {
    let parsed = try_parse_color(value).map_err(|_| ThemeDocumentError::InvalidTerminalColor {
        field: field.to_string(),
        value: value.to_string(),
    })?;
    let rgba = parsed.to_rgb();
    Ok(Rgb::new(
        (rgba.r.clamp(0.0, 1.0) * 255.0).round() as u8,
        (rgba.g.clamp(0.0, 1.0) * 255.0).round() as u8,
        (rgba.b.clamp(0.0, 1.0) * 255.0).round() as u8,
    ))
}

/// 把主题配置、终端色板与来源信息重新组合成可保存的主题文档。
pub(crate) fn serialize_theme_document(draft: &ThemeDraft) -> Result<String, ThemeDocumentError> {
    let mut theme = serde_json::to_value(&draft.config)
        .map_err(|source| ThemeDocumentError::Serialize { source })?;
    if let Some(palette) = draft.terminal_palette.as_ref() {
        theme["terminal"] = terminal_palette_value(palette);
    }
    let document = json!({
        "name": draft.set_name,
        "author": "transom-user",
        "themes": [theme],
    });
    serde_json::to_string_pretty(&document)
        .map_err(|source| ThemeDocumentError::Serialize { source })
}

fn terminal_palette_value(palette: &TerminalPalette) -> Value {
    let color = |value: Rgb| format!("#{:06X}", value.to_hex());
    json!({
        "background": color(palette.background),
        "foreground": color(palette.foreground),
        "foregroundIntense": palette.foreground_intense.map(color),
        "cursor": color(palette.cursor),
        "selection": color(palette.selection),
        "lineHighlight": color(palette.line_highlight),
        "findMatchBackground": color(palette.find_match_background),
        "findMatchBorder": color(palette.find_match_border),
        "ansi": palette.ansi.into_iter().map(color).collect::<Vec<_>>(),
    })
}

/// 把主题名收敛成可预测的文件名，防止路径穿越并保持跨平台可读性。
pub(crate) fn theme_file_stem(name: &str) -> String {
    let mut stem = String::with_capacity(name.len());
    let mut last_dash = false;
    let mut has_non_ascii = false;
    for ch in name.chars() {
        let accepted = ch.is_ascii_alphanumeric();
        if accepted {
            stem.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else {
            has_non_ascii |= !ch.is_ascii();
            if !last_dash && !stem.is_empty() {
                stem.push('-');
                last_dash = true;
            }
        }
    }
    while stem.ends_with('-') {
        stem.pop();
    }
    if stem.is_empty() || has_non_ascii {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        name.hash(&mut hasher);
        let suffix = hasher.finish();
        if stem.is_empty() {
            format!("custom-theme-{suffix:016x}")
        } else {
            format!("{stem}-{suffix:016x}")
        }
    } else {
        stem
    }
}

/// 确保用户主题名不会与现有主题重名；保存时只追加一次 Custom 后缀。
pub(crate) fn unique_theme_name(name: &str, existing: impl IntoIterator<Item = String>) -> String {
    let existing: BTreeSet<String> = existing.into_iter().collect();
    let trimmed = name.trim();
    let base = if trimmed.is_empty() {
        "Custom Theme"
    } else {
        trimmed
    };
    if !existing.contains(base) {
        return base.to_string();
    }
    let candidate = format!("{base} Custom");
    if !existing.contains(&candidate) {
        return candidate;
    }
    for suffix in 2..=999 {
        let candidate = format!("{base} Custom {suffix}");
        if !existing.contains(&candidate) {
            return candidate;
        }
    }
    format!("{base} Custom")
}

#[cfg(test)]
mod tests {
    use super::{
        ThemeDocumentError, parse_single_theme_document, serialize_theme_document, theme_file_stem,
        unique_theme_name,
    };
    use transom_core::TerminalPalette;

    const DOCUMENT: &str = r##"{
      "name": "test",
      "themes": [{
        "name": "测试主题",
        "mode": "dark",
        "colors": {
          "background": "#101010",
          "foreground": "#eeeeee",
          "border": "#333333",
          "primary.background": "#5599ff",
          "accent.background": "#334455"
        },
        "terminal": {
          "background": "#101010",
          "foreground": "#eeeeee",
          "cursor": "#ffffff",
          "selection": "#334455",
          "lineHighlight": "#202020",
          "findMatchBackground": "#224466",
          "findMatchBorder": "#5599ff",
          "ansi": ["#000000", "#ff0000", "#00ff00", "#ffff00", "#0000ff", "#ff00ff", "#00ffff", "#ffffff", "#111111", "#ff7777", "#77ff77", "#ffff77", "#7777ff", "#ff77ff", "#77ffff", "#ffffff"]
        }
      }]
    }"##;

    #[test]
    fn single_theme_round_trips_with_terminal_palette() {
        let draft = parse_single_theme_document(DOCUMENT).unwrap();
        assert_eq!(draft.config.name.as_ref(), "测试主题");
        assert_eq!(draft.terminal_palette.as_ref().unwrap().ansi.len(), 16);
        let serialized = serialize_theme_document(&draft).unwrap();
        let reparsed = parse_single_theme_document(&serialized).unwrap();
        assert_eq!(reparsed.config.name.as_ref(), "测试主题");
        assert_eq!(reparsed.terminal_palette, draft.terminal_palette);
    }

    #[test]
    fn legacy_terminal_palette_without_new_semantic_fields_still_loads() {
        let mut value: serde_json::Value = serde_json::from_str(DOCUMENT).unwrap();
        let terminal = value["themes"][0]["terminal"]
            .as_object_mut()
            .expect("terminal 为对象");
        for field in [
            "foregroundIntense",
            "lineHighlight",
            "findMatchBackground",
            "findMatchBorder",
        ] {
            terminal.remove(field);
        }
        let legacy = serde_json::to_string(&value).unwrap();
        let draft = parse_single_theme_document(&legacy).unwrap();
        let palette = draft.terminal_palette.expect("旧终端节应保留");
        assert_eq!(palette.foreground_intense, None);
        assert_eq!(
            palette.line_highlight,
            TerminalPalette::default().line_highlight
        );
        assert_eq!(
            palette.find_match_background,
            TerminalPalette::default().find_match_background
        );
        assert_eq!(
            palette.find_match_border,
            TerminalPalette::default().find_match_border
        );
    }

    #[test]
    fn rejects_multiple_themes_and_invalid_ansi_count() {
        let mut multiple: serde_json::Value = serde_json::from_str(DOCUMENT).unwrap();
        let theme = multiple["themes"][0].clone();
        multiple["themes"]
            .as_array_mut()
            .expect("themes 为数组")
            .push(theme);
        let multiple = serde_json::to_string(&multiple).unwrap();
        assert!(matches!(
            parse_single_theme_document(&multiple),
            Err(ThemeDocumentError::MultipleThemes { count: 2 })
        ));

        let mut invalid: serde_json::Value = serde_json::from_str(DOCUMENT).unwrap();
        invalid["themes"][0]["terminal"]["ansi"]
            .as_array_mut()
            .expect("ansi 为数组")
            .push(serde_json::json!("#ffffff"));
        let invalid = serde_json::to_string(&invalid).unwrap();
        assert!(matches!(
            parse_single_theme_document(&invalid),
            Err(ThemeDocumentError::InvalidAnsiCount { count: 17 })
        ));
    }

    #[test]
    fn file_stem_is_safe_and_theme_names_are_unique() {
        assert_eq!(theme_file_stem("../../Nya Theme"), "nya-theme");
        let chinese = theme_file_stem("玫瑰");
        assert!(chinese.starts_with("custom-theme-"));
        assert_ne!(chinese, theme_file_stem("主题"));
        assert_eq!(
            unique_theme_name("GitHub Dark", ["GitHub Dark".to_string()]),
            "GitHub Dark Custom"
        );
    }
}
