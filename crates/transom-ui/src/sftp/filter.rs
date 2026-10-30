//! 文件列表过滤的唯一实现：隐藏文件开关、排除规则与列内模糊查询。
//!
//! 设计原理（Why）：远端列、本地列与本地单栏面板的过滤规则必须一致，此前
//! 三处各写一份隐藏匹配与模糊匹配，本地单栏还漏掉了 `panel_hidden_patterns`。
//! 这里只依赖名称访问器，条目类型由调用方决定；父目录注入仍留在各自列表逻辑里。

use super::fuzzy_match_name;
use transom_core::TransferFilter;

/// 按名称过滤条目：先应用隐藏文件与排除规则，再应用列内模糊查询。
pub(crate) fn filter_named_entries<T, F>(
    entries: &[T],
    name_of: F,
    show_hidden: bool,
    query: &str,
    hidden_patterns: &str,
) -> Vec<T>
where
    T: Clone,
    F: Fn(&T) -> &str,
{
    let hidden_matcher = TransferFilter::from_text(hidden_patterns).matcher().ok();
    entries
        .iter()
        .filter(|entry| {
            let name = name_of(entry);
            (show_hidden || !name.starts_with('.'))
                && !hidden_matcher
                    .as_ref()
                    .is_some_and(|matcher| matcher.matches(name, false))
                && (query.is_empty() || fuzzy_match_name(name, query))
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::filter_named_entries;

    #[derive(Clone)]
    struct Entry(&'static str);

    fn names(entries: &[Entry]) -> Vec<&str> {
        entries.iter().map(|entry| entry.0).collect()
    }

    #[test]
    fn applies_hidden_exclusion_and_query() {
        let entries = [Entry("a.txt"), Entry(".env"), Entry("build.log")];
        assert_eq!(
            names(&filter_named_entries(
                &entries,
                |entry| entry.0,
                false,
                "",
                "*.log"
            )),
            vec!["a.txt"],
            "隐藏文件与排除规则都应生效"
        );
        assert_eq!(
            names(&filter_named_entries(
                &entries,
                |entry| entry.0,
                true,
                "env",
                ""
            )),
            vec![".env"]
        );
    }
}
