//! 从助手回复末尾提取交互式二选一 / 多选项确认列表。

/// 从模型回复中提取出的可交互选项列表。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedChoices {
    /// 引导用户选择的提示句。
    pub prompt: String,
    /// 是否允许多选（勾选多个选项后统一提交）。
    pub multi_select: bool,
    /// 提取到的选项正文（按顺序 2..=6 项）。
    pub options: Vec<String>,
}

const CHOICE_CUE_KEYWORDS: &[&str] = &[
    "二选一",
    "三选一",
    "四选一",
    "多选",
    "可多选",
    "选择以下",
    "以下选项",
    "选项中选择",
    "以下哪种",
    "以下哪些",
    "以下方案",
    "请选择",
    "请确认",
    "请指定",
    "需要你确认",
    "需要您确认",
    "choose",
    "select an option",
    "select all",
    "which option",
    "please confirm",
];

const MULTI_SELECT_KEYWORDS: &[&str] = &[
    "多选",
    "可多选",
    "选择多项",
    "以下哪些",
    "全部适用",
    "multiple",
    "select all",
];

fn has_choice_cue(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    CHOICE_CUE_KEYWORDS
        .iter()
        .any(|kw| line.contains(kw) || lower.contains(kw))
}

fn is_multi_select_cue(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    MULTI_SELECT_KEYWORDS
        .iter()
        .any(|kw| line.contains(kw) || lower.contains(kw))
}

fn parse_numbered_item(line: &str) -> Option<(usize, &str)> {
    let trimmed = line.trim_start();
    let digits_len = trimmed.bytes().take_while(|b| b.is_ascii_digit()).count();
    if digits_len == 0 || digits_len > 2 {
        return None;
    }
    let num = trimmed[..digits_len].parse::<usize>().ok()?;
    let rest = &trimmed[digits_len..];
    let body = rest
        .strip_prefix(". ")
        .or_else(|| rest.strip_prefix(") "))
        .or_else(|| rest.strip_prefix("、"))?;
    let body = body.trim();
    (!body.is_empty()).then_some((num, body))
}

/// 若助手回复末尾包含确认/选择引导语与 2..=6 个连续编号选项，则提取为交互式选择列表。
pub fn extract_prompt_choices(text: &str) -> Option<ExtractedChoices> {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() < 3 {
        return None;
    }
    let start = lines.len().saturating_sub(30);
    let tail = &lines[start..];

    let cue_idx = tail.iter().rposition(|l| has_choice_cue(l))?;
    let prompt = tail[cue_idx]
        .trim()
        .trim_start_matches('#')
        .trim()
        .to_owned();
    let multi_select = is_multi_select_cue(tail[cue_idx]);

    let mut options: Vec<String> = Vec::new();
    let mut expected_num = 1usize;
    let mut saw_blank_after_option = false;
    for raw in &tail[(cue_idx + 1)..] {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            if !options.is_empty() {
                saw_blank_after_option = true;
            }
            continue;
        }
        if let Some((num, body)) = parse_numbered_item(trimmed) {
            if num == expected_num {
                options.push(body.to_owned());
                expected_num += 1;
                saw_blank_after_option = false;
            } else {
                break;
            }
        } else if let Some(last) = options.last_mut() {
            if saw_blank_after_option || trimmed.starts_with('#') || trimmed.starts_with("```") {
                break;
            }
            last.push(' ');
            last.push_str(trimmed);
        }
    }

    (2..=6)
        .contains(&options.len())
        .then_some(ExtractedChoices {
            prompt,
            multi_select,
            options,
        })
}

#[cfg(test)]
mod tests {
    use super::extract_prompt_choices;

    #[test]
    fn extracts_chinese_two_option_prompt() {
        let reply = r#"## 需要你确认

请二选一，我收到后立即继续巡检：

1. 在 Transom 中连接目标主机后，告诉我"已连接"，我重新调用 `list_sessions` 获取 `session_id` 并执行完整巡检；
2. 直接提供已连接的 `session_id`。
"#;
        let extracted = extract_prompt_choices(reply).expect("应提取出二选一选项");
        assert_eq!(extracted.options.len(), 2);
        assert!(extracted.options[0].contains("在 Transom 中连接目标主机后"));
        assert!(extracted.options[1].contains("直接提供已连接的"));
    }

    #[test]
    fn ignores_ordinary_numbered_list_without_choice_cue() {
        let reply = "以下是巡检步骤：\n\n1. 检查 CPU 使用率\n2. 检查内存占用\n";
        assert!(extract_prompt_choices(reply).is_none());
    }

    #[test]
    fn detects_three_option_and_multi_select_prompts() {
        let multi = "请选择需要清理的目录（可多选）：\n1. /var/log\n2. /var/crash\n3. /tmp\n";
        let res = extract_prompt_choices(multi).expect("应识别多选列表");
        assert!(res.multi_select);
        assert_eq!(res.options.len(), 3);
    }

    #[test]
    fn extracts_select_from_options_with_trailing_note() {
        let reply = "请从以下选项中选择：\n\n1. 仅本机\n2. 全部 8 台 SSH 服务器\n3. 仅 Agree 组\n4. 仅 POC 组\n\n也可以直接告诉我具体某台服务器的 IP 或名称。\n";
        let res = extract_prompt_choices(reply).expect("应识别“请从以下选项中选择”并截断尾部说明");
        assert_eq!(res.options.len(), 4);
        assert_eq!(res.options[3], "仅 POC 组");
    }
}
