//! AI 消息流 Markdown 纯文本分段解析与增量缓存（零 GPUI 依赖，便于独立单测）。
//!
//! 设计原理（Why）：
//! - 将围栏分段状态机与 GPUI 渲染解耦，使得解析逻辑可作为纯算法独立维护与验证；
//! - [`StreamingMarkdownCache`] 对已闭合的代码块与稳定正文段落永不重解析，每次 `TextDelta`
//!   仅扫描最后一个未闭合尾部，将流式解析复杂度从 O(N²) 降至均摊 O(N)；
//! - `parse_markdown_segments` 直接复用增量缓存的状态机，杜绝双份围栏解析逻辑漂移。

/// Markdown 解析分段结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum MarkdownSegment {
    Prose(String),
    CodeBlock {
        lang: Option<String>,
        code: String,
        unclosed: bool,
    },
}

/// 流式 Markdown 增量解析缓存：已闭合的代码块与稳定正文段落永不重解析。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct StreamingMarkdownCache {
    segments: Vec<MarkdownSegment>,
    committed_count: usize,
    committed_bytes: usize,
}

impl StreamingMarkdownCache {
    /// 从已定稿的完整 Markdown 文本直接构造缓存。
    pub(super) fn from_complete(text: &str) -> Self {
        let mut cache = Self::default();
        cache.update(text);
        cache.committed_count = cache.segments.len();
        cache.committed_bytes = text.len();
        cache
    }

    /// 返回当前全部分段切片（零分配）。
    pub(super) fn as_slice(&self) -> &[MarkdownSegment] {
        &self.segments
    }

    /// 增量解析追加后的完整文本，仅重解析 `committed_bytes` 之后的尾部区间。
    pub(super) fn update(&mut self, full_text: &str) {
        if self.committed_bytes > full_text.len()
            || !full_text.is_char_boundary(self.committed_bytes)
        {
            *self = Self::default();
        }
        self.segments.truncate(self.committed_count);

        let tail = &full_text[self.committed_bytes..];
        let mut prose_lines: Vec<&str> = Vec::new();
        let mut active_fence: Option<(char, usize, Option<String>)> = None;
        let mut code_lines: Vec<&str> = Vec::new();
        let mut cursor = 0usize;
        let mut safe_commit_end = 0usize;

        for raw_line in tail.split_inclusive('\n') {
            let line_end = cursor + raw_line.len();
            let line = raw_line.strip_suffix('\n').unwrap_or(raw_line);
            let line = line.strip_suffix('\r').unwrap_or(line);

            if let Some((fence_ch, fence_len, _)) = &active_fence {
                if is_closing_fence(line, *fence_ch, *fence_len) {
                    if let Some((_, _, lang)) = active_fence.take() {
                        self.segments.push(MarkdownSegment::CodeBlock {
                            lang,
                            code: code_lines.join("\n"),
                            unclosed: false,
                        });
                    }
                    code_lines.clear();
                    // 只有遇到完整行结束符（\n）才推进提交边界，防止流式半行干扰
                    if raw_line.ends_with('\n') {
                        self.committed_count = self.segments.len();
                        safe_commit_end = line_end;
                    }
                } else {
                    code_lines.push(line);
                }
                cursor = line_end;
                continue;
            }

            if let Some((ch, len, lang)) = parse_opening_fence(line) {
                flush_prose(&mut prose_lines, &mut self.segments);
                if raw_line.ends_with('\n') {
                    self.committed_count = self.segments.len();
                    safe_commit_end = cursor;
                }
                active_fence = Some((ch, len, lang));
                cursor = line_end;
                continue;
            }

            prose_lines.push(line);
            cursor = line_end;
        }

        if let Some((_, _, lang)) = active_fence {
            self.segments.push(MarkdownSegment::CodeBlock {
                lang,
                code: code_lines.join("\n"),
                unclosed: true,
            });
        } else {
            flush_prose(&mut prose_lines, &mut self.segments);
        }
        self.committed_bytes += safe_commit_end;
    }
}

/// 判断代码语言是否为命令行 / Shell 类型。
pub(super) fn is_shell_language(lang: Option<&str>) -> bool {
    let Some(raw) = lang else {
        return false;
    };
    matches!(
        raw.trim().to_ascii_lowercase().as_str(),
        "sh" | "bash" | "zsh" | "shell" | "console" | "fish" | "powershell" | "pwsh" | "cmd"
    )
}

/// 将 Markdown 文本切分为正文段落与围栏代码块（容忍流式输出中未闭合的代码块）。
pub(super) fn parse_markdown_segments(input: &str) -> Vec<MarkdownSegment> {
    let mut cache = StreamingMarkdownCache::default();
    cache.update(input);
    cache.segments
}

fn flush_prose(lines: &mut Vec<&str>, out: &mut Vec<MarkdownSegment>) {
    if lines.is_empty() {
        return;
    }
    let text = lines.join("\n");
    lines.clear();
    if !text.trim().is_empty() {
        out.push(MarkdownSegment::Prose(text));
    }
}

fn parse_opening_fence(line: &str) -> Option<(char, usize, Option<String>)> {
    let trimmed = trim_up_to_3_spaces(line);
    let first = trimmed.chars().next()?;
    if first != '`' && first != '~' {
        return None;
    }
    let count = trimmed.chars().take_while(|&c| c == first).count();
    if count < 3 {
        return None;
    }
    let info = trimmed[count..].trim();
    if first == '`' && info.contains('`') {
        return None;
    }
    let lang = info
        .split_whitespace()
        .next()
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    Some((first, count, lang))
}

fn is_closing_fence(line: &str, ch: char, min_len: usize) -> bool {
    let trimmed = trim_up_to_3_spaces(line);
    let count = trimmed.chars().take_while(|&c| c == ch).count();
    count >= min_len && trimmed[count..].trim().is_empty()
}

fn trim_up_to_3_spaces(line: &str) -> &str {
    let mut idx = 0usize;
    for (n, b) in line.bytes().enumerate() {
        if b == b' ' && n < 3 {
            idx = n + 1;
        } else {
            break;
        }
    }
    &line[idx..]
}

#[cfg(test)]
mod tests {
    use super::{
        MarkdownSegment, StreamingMarkdownCache, is_shell_language, parse_markdown_segments,
    };

    #[test]
    fn incremental_cache_matches_full_parse_across_chunk_boundaries() {
        let md = "第一段诊断：\n\n```bash\ndf -h\nfree -m\n```\n\n第二段总结：\n~~~json\n{\"ok\":true}\n~~~\n完成。";
        let mut cache = StreamingMarkdownCache::default();
        let mut acc = String::new();
        for ch in md.chars() {
            acc.push(ch);
            cache.update(&acc);
            assert_eq!(cache.as_slice(), parse_markdown_segments(&acc).as_slice());
        }
        assert_eq!(cache.as_slice(), parse_markdown_segments(md).as_slice());
    }

    #[test]
    fn parses_prose_and_multiple_code_blocks() {
        let md = "先检查磁盘：\n\n```bash\ndf -h\nfree -m\n```\n\n再看日志：\n~~~json\n{\"ok\":true}\n~~~\n";
        let segs = parse_markdown_segments(md);
        assert_eq!(segs.len(), 4);
        assert_eq!(segs[0], MarkdownSegment::Prose("先检查磁盘：\n".to_owned()));
        assert_eq!(
            segs[1],
            MarkdownSegment::CodeBlock {
                lang: Some("bash".to_owned()),
                code: "df -h\nfree -m".to_owned(),
                unclosed: false,
            }
        );
        assert_eq!(segs[2], MarkdownSegment::Prose("\n再看日志：".to_owned()));
        assert_eq!(
            segs[3],
            MarkdownSegment::CodeBlock {
                lang: Some("json".to_owned()),
                code: "{\"ok\":true}".to_owned(),
                unclosed: false,
            }
        );
    }

    #[test]
    fn keeps_unclosed_streaming_fence_isolated() {
        let md = "正在输出脚本：\n```sh\necho hello\nls -la";
        let segs = parse_markdown_segments(md);
        assert_eq!(segs.len(), 2);
        assert_eq!(
            segs[1],
            MarkdownSegment::CodeBlock {
                lang: Some("sh".to_owned()),
                code: "echo hello\nls -la".to_owned(),
                unclosed: true,
            }
        );
    }

    #[test]
    fn detects_shell_languages_case_insensitively() {
        assert!(is_shell_language(Some("bash")));
        assert!(is_shell_language(Some("SH")));
        assert!(is_shell_language(Some("PowerShell")));
        assert!(!is_shell_language(Some("rust")));
        assert!(!is_shell_language(None));
    }
}
