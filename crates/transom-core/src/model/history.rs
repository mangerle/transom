//! 命令历史：采集到的命令行的收录与检索策略。
//!
//! 设计原理（Why）：
//! - 实现初衷：建议功能要"按前缀取最近用过的命令"，这份策略与存储介质无关，
//!   放在 core 里可以纯单测，界面与持久化都只调用它；
//! - 核心优势：列表按"最近使用优先"排序且天然去重，检索是前缀匹配，
//!   复杂度 O(n)（n 上限 2000，人速按键下完全无感）；
//! - 代价与局限：全局一份历史（不按主机分列），因此在 A 机敲过的
//!   `ssh root@10.0.0.8` 会在 B 机的输入框里也被建议——这与 shell 自身的
//!   全局历史行为一致，若要按主机分列需要另一份按会话存储的结构。

/// 历史条数上限：再大也不会被建议使用到（前缀命中通常只有几十条），
/// 却会让每次写盘的文件持续变大。
pub const HISTORY_LIMIT: usize = 2000;

/// 一次建议最多返回的条数，与浮层高度匹配。
pub const SUGGESTION_LIMIT: usize = 6;

/// 收录一条命令：去重（把重复项提到最前）并按上限截断。
///
/// 返回是否实际记录。空串、单字符、以空格开头的行（`HIST_IGNORE_SPACE` 约定，
/// 用户用它表示"这条别留档"）都不记录；含换行的粘贴残留会被压成一行。
pub fn record(list: &mut Vec<String>, command: &str) -> bool {
    // 前置空格要在归一化之前判：归一化会把首尾空白吃掉，之后就再也看不出来。
    if command.starts_with([' ', '\t']) {
        return false;
    }
    let command = normalize_command(command);
    if command.chars().count() < 2 {
        return false;
    }
    if let Some(index) = list.iter().position(|existing| *existing == command) {
        list.remove(index);
    }
    list.insert(0, command);
    list.truncate(HISTORY_LIMIT);
    true
}

/// 按前缀取最近用过的命令。
pub fn suggestions(list: &[String], prefix: &str, limit: usize) -> Vec<String> {
    if prefix.is_empty() {
        return Vec::new();
    }
    list.iter()
        .filter(|command| command.starts_with(prefix) && command.as_str() != prefix)
        .take(limit.max(1))
        .cloned()
        .collect()
}

/// 把采集到的原始行压成适合存档的单行文本。
fn normalize_command(command: &str) -> String {
    command
        .split(['\r', '\n'])
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_most_recent_first_and_dedupes() {
        let mut list = Vec::new();
        assert!(record(&mut list, "git status"));
        assert!(record(&mut list, "ls -la"));
        assert!(record(&mut list, "git status"));
        assert_eq!(list, vec!["git status".to_string(), "ls -la".to_string()]);
    }

    /// 前置空格是 shell 里的"别记这条"约定，必须尊重。
    #[test]
    fn skips_blank_short_and_leading_space() {
        let mut list = Vec::new();
        assert!(!record(&mut list, ""));
        assert!(!record(&mut list, "x"));
        assert!(!record(&mut list, "  secret-token abc"));
        assert!(!record(&mut list, "\r\n"));
        assert!(list.is_empty());
    }

    /// 粘贴多行时取到的可能是带换行的一坨；压成一行才不会把历史文件写坏。
    #[test]
    fn multiline_input_collapses_to_one_line() {
        let mut list = Vec::new();
        assert!(record(&mut list, "for i in 1 2 3; do\r\n  echo $i;\ndone"));
        assert_eq!(list.len(), 1);
        assert!(!list[0].contains('\n') && !list[0].contains('\r'));
        assert!(list[0].starts_with("for i in"));
    }

    #[test]
    fn truncates_at_limit() {
        let mut list = Vec::new();
        for index in 0..(HISTORY_LIMIT + 50) {
            record(&mut list, &format!("cmd{index} arg"));
        }
        assert_eq!(list.len(), HISTORY_LIMIT);
        // 最新的一条在开头，最旧的已被挤出。
        assert_eq!(list[0], format!("cmd{} arg", HISTORY_LIMIT + 49));
    }

    #[test]
    fn suggestions_are_prefix_matched_and_exclude_the_exact_line() {
        let list = vec![
            "git commit -m x".to_string(),
            "git status".to_string(),
            "grep foo".to_string(),
        ];
        // 前缀命中按"最近使用"的列表顺序返回，且不重复当前已敲完的那行。
        assert_eq!(
            suggestions(&list, "git ", 6),
            vec!["git commit -m x".to_string(), "git status".to_string()]
        );
        assert!(suggestions(&list, "git status", 6).is_empty());
        assert!(suggestions(&list, "", 6).is_empty());
    }

    #[test]
    fn suggestion_limit_is_respected() {
        let list: Vec<String> = (0..20).map(|index| format!("log{index}")).collect();
        assert_eq!(suggestions(&list, "log", 3).len(), 3);
        // 传 0 也要返回至少一条可选项的可能，不能因为参数退化成空查询。
        assert_eq!(suggestions(&list, "log", 0).len(), 1);
    }
}
