//! 命令级 AST / 链路解析策略、执行前影响说明与用户规则引擎。
//!
//! 设计原理（Why）：
//! - 单纯按空格或裸字符串匹配容易被 `ls && rm -rf /`、`;`、`$(...)`、反引号等链式结构绕过；
//! - 本模块实现引号与转义感知的 Shell 片段拆解器 [`split_shell_segments`]，对每段独立定级并取最严风险；
//! - 嵌套命令替换具有固定深度上限，超过上限时保留原始片段交给失败关闭策略处理。
//! - 同时支持用户配置的命令前缀规则表（放行/拒绝/询问）、按作用域隔离的审批授权，以及高危命令执行前的影响摘要。

use serde::{Deserialize, Serialize};

use crate::approval::{ApprovalGrantStore, ApprovalScope};
use crate::policy::{ToolRisk, assess_shell_command};

/// 用户自定义的命令策略动作。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecPolicyAction {
    /// 命中该前缀时自动放行（仍受 ReadOnly 主机和 Destructive 兜底约束）。
    Allow,
    /// 命中该前缀时直接拦截。
    Deny,
    /// 命中该前缀时强制人工审批。
    #[default]
    Ask,
}

impl ExecPolicyAction {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Allow => "放行",
            Self::Deny => "拦截",
            Self::Ask => "询问",
        }
    }
}

/// 持久化存储的用户命令前缀策略规则。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecPolicyRule {
    #[serde(default)]
    pub id: String,
    /// 命令前缀模式（如 `docker ps`、`systemctl restart`、`rm -rf`）。
    #[serde(default)]
    pub pattern: String,
    #[serde(default)]
    pub action: ExecPolicyAction,
    #[serde(default)]
    pub note: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

/// 链式命令策略评估结果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChainEvalResult {
    /// 所有拆解片段中的最高风险等级。
    pub worst_risk: ToolRisk,
    /// 若任意片段命中了 Deny 规则，返回拦截原因。
    pub denied_reason: Option<String>,
    /// 所有片段是否均命中用户 `Allow` 规则或当前审批作用域授权（且无 `Destructive` 片段）。
    pub all_allowlisted: bool,
    /// 是否强制要求人工确认（命中 `Ask` 规则或存在不可自动放行片段）。
    pub force_ask: bool,
    /// 是否存在无法证明安全的命令片段。
    pub requires_confirmation: bool,
}

/// 引号与转义感知的 Shell 命令分段拆解器。
///
/// 拆分 `&&`、`||`、`;`、`|`、换行，并提取 `$(...)` 与反引号中的内嵌命令。
/// 嵌套深度超过上限时保留原始片段，由风险分级按未知命令强制确认。
pub fn split_shell_segments(command: &str) -> Vec<String> {
    split_shell_segments_inner(command, 0)
}

const MAX_SHELL_NESTING_DEPTH: usize = 16;

fn split_shell_segments_inner(command: &str, depth: usize) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = command.chars().collect();
    let len = chars.len();
    let mut i = 0usize;
    let mut in_single = false;
    let mut in_double = false;

    while i < len {
        let ch = chars[i];
        if in_single {
            current.push(ch);
            if ch == '\'' {
                in_single = false;
            }
            i += 1;
            continue;
        }

        if ch == '\\' && i + 1 < len {
            current.push(ch);
            current.push(chars[i + 1]);
            i += 2;
            continue;
        }

        if ch == '"' {
            in_double = !in_double;
            current.push(ch);
            i += 1;
            continue;
        }

        if !in_double && ch == '\'' {
            in_single = true;
            current.push(ch);
            i += 1;
            continue;
        }

        // 提取 $(...) 内嵌子命令
        if ch == '$' && i + 1 < len && chars[i + 1] == '(' {
            let (inner, next_idx) = extract_paren_subcommand(&chars, i + 2);
            if !inner.trim().is_empty() {
                if depth < MAX_SHELL_NESTING_DEPTH {
                    segments.extend(split_shell_segments_inner(&inner, depth + 1));
                } else {
                    segments.push(inner);
                }
            }
            current.push_str("$(...)");
            i = next_idx;
            continue;
        }

        // 提取 `...` 反引号内嵌子命令
        if ch == '`' {
            let (inner, next_idx) = extract_backtick_subcommand(&chars, i + 1);
            if !inner.trim().is_empty() {
                if depth < MAX_SHELL_NESTING_DEPTH {
                    segments.extend(split_shell_segments_inner(&inner, depth + 1));
                } else {
                    segments.push(inner);
                }
            }
            current.push_str("`...`");
            i = next_idx;
            continue;
        }

        if !in_double {
            let is_and = ch == '&' && i + 1 < len && chars[i + 1] == '&';
            let is_or = ch == '|' && i + 1 < len && chars[i + 1] == '|';
            if is_and || is_or {
                push_trimmed(&mut segments, &mut current);
                i += 2;
                continue;
            }
            if matches!(ch, ';' | '|' | '\n') {
                push_trimmed(&mut segments, &mut current);
                i += 1;
                continue;
            }
        }

        current.push(ch);
        i += 1;
    }

    push_trimmed(&mut segments, &mut current);
    segments
}

fn push_trimmed(out: &mut Vec<String>, buf: &mut String) {
    let trimmed = buf.trim();
    if !trimmed.is_empty() {
        out.push(trimmed.to_owned());
    }
    buf.clear();
}

fn extract_paren_subcommand(chars: &[char], start: usize) -> (String, usize) {
    let mut depth = 1usize;
    let mut i = start;
    let mut out = String::new();
    while i < chars.len() {
        let ch = chars[i];
        if ch == '(' {
            depth += 1;
        } else if ch == ')' {
            depth -= 1;
            if depth == 0 {
                return (out, i + 1);
            }
        }
        out.push(ch);
        i += 1;
    }
    (out, chars.len())
}

fn extract_backtick_subcommand(chars: &[char], start: usize) -> (String, usize) {
    let mut i = start;
    let mut out = String::new();
    while i < chars.len() {
        let ch = chars[i];
        if ch == '`' {
            return (out, i + 1);
        }
        out.push(ch);
        i += 1;
    }
    (out, chars.len())
}

/// 剥离前导 `sudo` 与环境变量赋值（如 `LANG=C`），返回规范化后的命令前缀用于策略匹配。
fn normalize_segment_for_match(seg: &str) -> String {
    let mut tokens: Vec<&str> = seg.split_whitespace().collect();
    while let Some(&first) = tokens.first() {
        if first == "sudo" || (first.contains('=') && !first.starts_with('-')) {
            tokens.remove(0);
        } else {
            break;
        }
    }
    tokens.join(" ")
}

fn matches_prefix(normalized_seg: &str, pattern: &str) -> bool {
    let pat = pattern.trim();
    if pat.is_empty() {
        return false;
    }
    let seg_lower = normalized_seg.to_ascii_lowercase();
    let pat_lower = pat.to_ascii_lowercase();
    seg_lower == pat_lower || seg_lower.starts_with(&format!("{pat_lower} "))
}

/// 对整条命令及其全部子片段执行策略评估。
pub fn evaluate_command_chain(
    command: &str,
    rules: &[ExecPolicyRule],
    approval_grants: &ApprovalGrantStore,
    approval_scope: &ApprovalScope,
) -> ChainEvalResult {
    let segments = split_shell_segments(command);
    if segments.is_empty() {
        return ChainEvalResult {
            worst_risk: ToolRisk::Write,
            denied_reason: None,
            all_allowlisted: false,
            force_ask: false,
            requires_confirmation: true,
        };
    }

    let mut worst_risk = ToolRisk::SafeRead;
    let mut all_allowlisted = true;
    let mut force_ask = false;
    let mut requires_confirmation = false;

    for seg in &segments {
        let assessment = assess_shell_command(seg);
        let seg_risk = assessment.risk;
        requires_confirmation |= assessment.requires_confirmation;
        if seg_risk > worst_risk {
            worst_risk = seg_risk;
        }

        let norm = normalize_segment_for_match(seg);
        let mut allow_rule = false;
        let mut segment_ask = false;

        for rule in rules.iter().filter(|r| r.enabled) {
            if matches_prefix(&norm, &rule.pattern) {
                match rule.action {
                    ExecPolicyAction::Deny => {
                        let reason = if rule.note.trim().is_empty() {
                            format!("命令片段 `{seg}` 命中拦截规则 `{}`", rule.pattern)
                        } else {
                            format!(
                                "命令片段 `{seg}` 命中拦截规则 `{}`：{}",
                                rule.pattern, rule.note
                            )
                        };
                        return ChainEvalResult {
                            worst_risk,
                            denied_reason: Some(reason),
                            all_allowlisted: false,
                            force_ask: false,
                            requires_confirmation,
                        };
                    }
                    ExecPolicyAction::Ask => {
                        force_ask = true;
                        segment_ask = true;
                    }
                    ExecPolicyAction::Allow => {
                        allow_rule = true;
                    }
                }
            }
        }

        let granted = approval_grants.permits(approval_scope, &norm);
        let seg_allowed =
            !segment_ask && seg_risk != ToolRisk::Destructive && (allow_rule || granted);

        if !seg_allowed {
            all_allowlisted = false;
        }
    }

    if worst_risk == ToolRisk::Destructive {
        all_allowlisted = false;
    }

    ChainEvalResult {
        worst_risk,
        denied_reason: None,
        all_allowlisted,
        force_ask,
        requires_confirmation,
    }
}

/// 提取适合展示在审批卡片“本会话允许同类”按钮中的安全命令前缀。
///
/// 高危命令返回 `None`，避免用户误将破坏性前缀加入白名单。
pub fn extract_command_prefix(command: &str) -> Option<String> {
    let segments = split_shell_segments(command);
    if segments.len() != 1 {
        return None;
    }
    let assessment = assess_shell_command(&segments[0]);
    if assessment.risk == ToolRisk::Destructive || assessment.requires_confirmation {
        return None;
    }
    let norm = normalize_segment_for_match(&segments[0]);
    let parts: Vec<&str> = norm.split_whitespace().collect();
    let first = *parts.first()?;
    let prefix = match (first, parts.get(1).copied()) {
        (
            "docker" | "podman" | "kubectl" | "systemctl" | "git" | "cargo" | "npm" | "apt" | "yum"
            | "dnf",
            Some(sub),
        ) if !sub.starts_with('-') => {
            format!("{first} {sub}")
        }
        _ => first.to_owned(),
    };
    Some(prefix)
}

/// 为高危或变更命令生成简明中文影响面说明（Explain before Run）。
pub fn explain_command_impact(command: &str) -> Option<String> {
    let lower = command.to_ascii_lowercase();
    let mut impacts = Vec::new();
    if lower.contains("rm -rf") || lower.contains("rm -fr") {
        impacts.push("递归强制删除指定路径且不可恢复");
    }
    if lower.contains("mkfs") || lower.contains("dd if=") {
        impacts.push("直接格式化或覆写块设备数据，可能导致数据永久丢失");
    }
    if lower.contains("shutdown") || lower.contains("reboot") || lower.contains("poweroff") {
        impacts.push("将重启或关闭目标主机，中断所有在线连接与运行中服务");
    }
    if lower.contains("systemctl restart") || lower.contains("systemctl stop") {
        impacts.push("将重启或停止系统服务，期间请求可能短暂中断");
    }
    if lower.contains("docker rm")
        || lower.contains("docker stop")
        || lower.contains("compose down")
    {
        impacts.push("将停止或销毁运行中的容器实例");
    }
    if lower.contains("chmod ") || lower.contains("chown ") {
        impacts.push("将修改文件或目录权限/归属，可能影响服务访问控制");
    }
    if lower.contains('>') && !lower.contains("/dev/null") {
        impacts.push("包含输出重定向，可能覆写或追加目标文件内容");
    }
    if impacts.is_empty() {
        None
    } else {
        Some(impacts.join("；"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::approval::{ApprovalGrantStore, ApprovalScope, HostProtection};

    fn global_scope() -> ApprovalScope {
        ApprovalScope::new("chat-1", None, HostProtection::Normal)
    }

    #[test]
    fn split_segments_handles_quotes_and_subshells() {
        let segs = split_shell_segments(r#"echo "a && b" && ls -la; $(rm -rf /tmp/x) | head"#);
        assert!(segs.iter().any(|s| s.contains("echo \"a && b\"")));
        assert!(segs.iter().any(|s| s == "ls -la"));
        assert!(segs.iter().any(|s| s == "rm -rf /tmp/x"));
        assert!(segs.iter().any(|s| s == "head"));
    }

    #[test]
    fn deeply_nested_substitutions_are_bounded() {
        let command = format!("{}echo ok{}", "$(".repeat(64), ")".repeat(64));
        let segments = split_shell_segments(&command);
        assert!(!segments.is_empty());
        assert!(segments.len() <= 66);
    }

    #[test]
    fn chained_destructive_command_is_caught() {
        let res = evaluate_command_chain(
            "ls -la && rm -rf /var/tmp/test",
            &[],
            &ApprovalGrantStore::new(),
            &global_scope(),
        );
        assert_eq!(res.worst_risk, ToolRisk::Destructive);
        assert!(!res.all_allowlisted);
    }

    #[test]
    fn deny_and_allow_rules_work_per_segment() {
        let rules = vec![
            ExecPolicyRule {
                id: "r1".into(),
                pattern: "docker ps".into(),
                action: ExecPolicyAction::Allow,
                note: String::new(),
                enabled: true,
            },
            ExecPolicyRule {
                id: "r2".into(),
                pattern: "rm -rf".into(),
                action: ExecPolicyAction::Deny,
                note: "禁止批量删除".into(),
                enabled: true,
            },
        ];
        let ok = evaluate_command_chain(
            "docker ps -a",
            &rules,
            &ApprovalGrantStore::new(),
            &global_scope(),
        );
        assert!(ok.denied_reason.is_none());
        assert!(ok.all_allowlisted);

        let blocked = evaluate_command_chain(
            "docker ps && rm -rf /tmp/a",
            &rules,
            &ApprovalGrantStore::new(),
            &global_scope(),
        );
        assert!(blocked.denied_reason.is_some());
    }

    #[test]
    fn prefix_and_impact_extraction_behave_safely() {
        assert_eq!(
            extract_command_prefix("sudo systemctl restart nginx"),
            Some("systemctl restart".to_owned())
        );
        assert_eq!(extract_command_prefix("rm -rf /tmp/foo"), None);
        assert!(explain_command_impact("systemctl restart nginx").is_some());
    }
}
