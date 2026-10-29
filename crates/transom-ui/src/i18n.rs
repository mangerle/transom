//! 界面语言解析：显式选择优先，空值跟随系统。
//!
//! 设计原理（Why）：配置里的 `language` 允许为空，表示"跟随系统"。这样新建
//! 配置不会把某一门语言写死（开源分发时英文用户首次打开不会看到中文），而旧
//! 配置里的显式值仍原样保留，不会被静默翻转。桌面端的 locale 在 Linux 上通过
//! `LC_ALL`/`LC_MESSAGES`/`LANG` 依次读取，非中文一律回退英文；当前只维护中英
//! 两套资源，不做完整 BCP 47 匹配。

/// 解析最终生效的界面语言。
///
/// 显式配置非空时原样使用（便于以后新增语言）；空值读取系统 locale。
pub(crate) fn resolve_language(configured: &str) -> String {
    let configured = configured.trim();
    if !configured.is_empty() {
        return configured.to_string();
    }
    system_language()
}

/// 读取系统语言并按现有资源归一化。
fn system_language() -> String {
    for key in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(value) = std::env::var(key) {
            let value = value.trim();
            if !value.is_empty() {
                return normalize_locale(value);
            }
        }
    }
    "en".to_string()
}

/// 系统 locale 到受支持语言的归一化：中文（含 zh_TW/zh-HK 等）统一用简体资源，
/// 其余一律英文。`C`/`POSIX` 这类无语言信息的取值也落在英文。
fn normalize_locale(raw: &str) -> String {
    if raw.to_ascii_lowercase().starts_with("zh") {
        "zh-CN".to_string()
    } else {
        "en".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_locale;

    #[test]
    fn test_normalize_locale_maps_chinese_variants() {
        assert_eq!(normalize_locale("zh_CN.UTF-8"), "zh-CN");
        assert_eq!(normalize_locale("zh_TW.UTF-8"), "zh-CN");
        assert_eq!(normalize_locale("ZH"), "zh-CN");
    }

    #[test]
    fn test_normalize_locale_falls_back_to_english() {
        assert_eq!(normalize_locale("en_US.UTF-8"), "en");
        assert_eq!(normalize_locale("de_DE.UTF-8"), "en");
        assert_eq!(normalize_locale("C"), "en");
        assert_eq!(normalize_locale("POSIX"), "en");
    }
}
