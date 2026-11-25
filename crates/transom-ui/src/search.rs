//! 界面列表的通用搜索匹配。
//!
//! 设计原理（Why）：侧栏、容器列表和日志过滤共享“查询已预先转为小写”的扫描
//! 口径。ASCII 是绝大多数主机名、镜像名和文件夹名的形态，直接在字节窗口上
//! 比较可避免每个条目、每个字段创建临时小写字符串；非 ASCII 才回退到完整折叠。

/// 判断文本是否包含预先转为小写的查询串。
pub(crate) fn contains_case_insensitive(text: &str, query_lower: &str) -> bool {
    if query_lower.is_empty() {
        return true;
    }
    if text.is_ascii() && query_lower.is_ascii() {
        return text
            .as_bytes()
            .windows(query_lower.len())
            .any(|window| window.eq_ignore_ascii_case(query_lower.as_bytes()));
    }
    text.to_lowercase().contains(query_lower)
}

/// 任一字段命中查询即返回真；空查询恒为真。
pub(crate) fn any_field_matches<'a>(
    query_lower: &str,
    fields: impl IntoIterator<Item = &'a str>,
) -> bool {
    query_lower.is_empty()
        || fields
            .into_iter()
            .any(|field| contains_case_insensitive(field, query_lower))
}

#[cfg(test)]
mod tests {
    use super::{any_field_matches, contains_case_insensitive};

    #[test]
    fn ascii_search_ignores_case() {
        assert!(contains_case_insensitive("Prod-DB", "prod"));
        assert!(contains_case_insensitive("Prod-DB", "db"));
        assert!(!contains_case_insensitive("Prod-DB", "staging"));
    }

    #[test]
    fn unicode_search_uses_case_folding_fallback() {
        assert!(contains_case_insensitive("ÜBER-SERVICE", "über"));
    }

    #[test]
    fn any_field_matches_or_empty_query() {
        assert!(any_field_matches("nginx", ["Web", "nginx:latest"]));
        assert!(!any_field_matches("redis", ["Web", "nginx:latest"]));
        assert!(any_field_matches("", [] as [&str; 0]));
    }
}
