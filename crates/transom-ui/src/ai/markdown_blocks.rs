//! AI 消息流 Markdown 分段解析与代码/命令块卡片渲染。
//!
//! 设计原理（Why）：
//! - `TextView::markdown` 在流式未闭合围栏或纯代码块场景下缺少复制、填入终端与语言标签交互；
//! - 先将 Markdown 按围栏（```` ``` ```` / `~~~`）切分为 [`MarkdownSegment::Prose`] 与
//!   [`MarkdownSegment::CodeBlock`]，普通正文继续交给 `TextView::markdown` 渲染（保留标题/列表/表格/选中），
//!   代码块则渲染为带语言图标、一键复制、Shell 填入终端与存为快捷指令的独立卡片。

use std::rc::Rc;
use std::time::Duration;

use gpui_kit::base::{TextView, TextViewMotion, TextViewStyle, v_flex};
use gpui_kit::*;

use crate::palette::Palette;
use crate::theme::tokens::space;

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

/// 流式 Markdown 增量解析缓存：已闭合的代码块与稳定正文段落永不重解析，
/// 每次 `TextDelta` 仅解析最后一个未闭合的尾部片段，将流式复杂度从 O(N²) 降至均摊 O(N)。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct StreamingMarkdownCache {
    segments: Vec<MarkdownSegment>,
    committed_count: usize,
    committed_bytes: usize,
}

impl StreamingMarkdownCache {
    /// 从已定稿的完整 Markdown 文本直接构造缓存。
    pub(super) fn from_complete(text: &str) -> Self {
        let segments = parse_markdown_segments(text);
        let committed_count = segments.len();
        Self {
            segments,
            committed_count,
            committed_bytes: text.len(),
        }
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
                    let (_, _, lang) = active_fence.take().expect("checked");
                    self.segments.push(MarkdownSegment::CodeBlock {
                        lang,
                        code: code_lines.join("\n"),
                        unclosed: false,
                    });
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

/// 带键与文本双参数的回调类型（用于复制、存为快捷指令）。
pub(super) type KeyTextCallback = Rc<dyn Fn(String, String, &mut Window, &mut App)>;
/// 单文本参数回调类型（用于填入终端）。
pub(super) type TextCallback = Rc<dyn Fn(String, &mut Window, &mut App)>;
/// 远端文件快照还原回调类型（参数：主机名、目标路径、本机备份路径）。
pub(super) type RevertFileCallback = Rc<dyn Fn(String, String, String, &mut Window, &mut App)>;

/// 代码块渲染所需的回调集合。
#[derive(Clone)]
pub(super) struct CodeBlockCallbacks {
    pub(super) on_copy: KeyTextCallback,
    pub(super) on_send_to_terminal: TextCallback,
    pub(super) on_save_snippet: KeyTextCallback,
    pub(super) on_revert_remote_file: RevertFileCallback,
    pub(super) on_jump_to_terminal: TextCallback,
    pub(super) on_subagent_action: KeyTextCallback,
}

/// 渲染 Markdown 正文的入参对象。
pub(super) struct MarkdownRenderParams<'a> {
    pub(super) msg_id: &'a str,
    pub(super) text: &'a str,
    pub(super) cached_segments: Option<&'a [MarkdownSegment]>,
    pub(super) streaming: bool,
    pub(super) copied_key: Option<&'a str>,
    pub(super) has_active_terminal: bool,
    pub(super) p: &'a Palette,
    pub(super) callbacks: &'a CodeBlockCallbacks,
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
    let mut segments = Vec::new();
    let mut prose_lines: Vec<&str> = Vec::new();
    let mut active_fence: Option<(char, usize, Option<String>)> = None;
    let mut code_lines: Vec<&str> = Vec::new();

    for line in input.lines() {
        if let Some((fence_ch, fence_len, _)) = &active_fence {
            if is_closing_fence(line, *fence_ch, *fence_len) {
                if let Some((_, _, lang)) = active_fence.take() {
                    segments.push(MarkdownSegment::CodeBlock {
                        lang,
                        code: code_lines.join("\n"),
                        unclosed: false,
                    });
                }
                code_lines.clear();
            } else {
                code_lines.push(line);
            }
            continue;
        }

        if let Some((ch, len, lang)) = parse_opening_fence(line) {
            flush_prose(&mut prose_lines, &mut segments);
            active_fence = Some((ch, len, lang));
            continue;
        }
        prose_lines.push(line);
    }

    if let Some((_, _, lang)) = active_fence {
        segments.push(MarkdownSegment::CodeBlock {
            lang,
            code: code_lines.join("\n"),
            unclosed: true,
        });
    } else {
        flush_prose(&mut prose_lines, &mut segments);
    }
    segments
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

/// 构造对标现代文档/Agent 信息流的 Markdown 排版样式：
/// 收窄标题字号梯度、为行内代码与引用块提供胶囊/导轨层次，并为表格与选中态提供一致的语义色。
fn build_markdown_style(p: &Palette) -> TextViewStyle {
    let mut table = StyleRefinement::default();
    table.overflow.x = Some(Overflow::Scroll);
    table = table
        .rounded(p.radius.md)
        .border_1()
        .border_color(p.stroke.subtle);

    let table_head = StyleRefinement::default()
        .bg(p.surface.subtle)
        .text_color(p.foreground)
        .font_weight(FontWeight::MEDIUM);

    let table_cell = StyleRefinement::default()
        .px(px(12.0))
        .py(px(8.0))
        .border_color(p.stroke.faint);

    let blockquote_style = StyleRefinement::default()
        .border_l_2()
        .border_color(p.primary.opacity(0.45))
        .bg(p.surface.faint)
        .px(rems(space::SM))
        .py(rems(space::XS))
        .rounded(p.radius.sm)
        .text_color(p.text.subtle);

    let inline_code_style = StyleRefinement::default()
        .font_family(p.mono_font_family.clone())
        .text_size(rems(0.84))
        .px(px(5.0))
        .py(px(1.5))
        .rounded(px(4.0))
        .bg(p.surface.subtle)
        .border_1()
        .border_color(p.stroke.faint);

    let list_style = StyleRefinement::default().text_color(p.text.muted);

    TextViewStyle::default()
        .with_foreground(p.foreground)
        .with_muted_foreground(p.muted_foreground)
        .with_link(p.primary)
        .with_selection(p.primary.opacity(0.22))
        .with_border(p.stroke.faint)
        .with_code_background(p.surface.faint)
        .with_dark(p.background.l < 0.5)
        .with_paragraph_gap(rems(0.6))
        .with_heading(|level| {
            let (size, weight) = match level {
                1 => (px(17.0), FontWeight::SEMIBOLD),
                2 => (px(15.5), FontWeight::SEMIBOLD),
                3 => (px(14.5), FontWeight::MEDIUM),
                _ => (px(13.5), FontWeight::MEDIUM),
            };
            StyleRefinement::default()
                .text_size(size)
                .font_weight(weight)
        })
        .with_table(table)
        .with_table_head(table_head)
        .with_table_cell(table_cell)
        .with_table_row_hover(p.surface.ghost)
        .with_blockquote_style(blockquote_style)
        .with_inline_code_style(inline_code_style)
        .with_list_style(list_style)
        .with_inline_code(HighlightStyle {
            background_color: Some(p.surface.subtle),
            ..Default::default()
        })
}

/// 将解析后的 Markdown 分段渲染为混排正文与代码块卡片。
pub(super) fn render_markdown_content(params: MarkdownRenderParams<'_>) -> AnyElement {
    let owned;
    let segments: &[MarkdownSegment] = if let Some(cached) = params.cached_segments {
        cached
    } else {
        owned = parse_markdown_segments(params.text);
        &owned
    };
    if segments.is_empty() {
        return div().into_any_element();
    }
    let md_style = build_markdown_style(params.p);
    let mut col = v_flex().w_full().gap(rems(space::SM));
    for (idx, seg) in segments.iter().enumerate() {
        match seg {
            MarkdownSegment::Prose(prose) => {
                let mut tv = TextView::markdown(
                    SharedString::from(format!("ai-md-{}-{idx}", params.msg_id)),
                    prose.clone(),
                )
                .style(md_style.clone())
                .selectable(true);
                if params.streaming {
                    tv = tv.motion(
                        TextViewMotion::default()
                            .with_stream_fade(Duration::from_millis(160))
                            .with_stream_fade_stagger(Duration::from_millis(22)),
                    );
                }
                col = col.child(div().w_full().line_height(relative(1.62)).child(tv));
            }
            MarkdownSegment::CodeBlock {
                lang,
                code,
                unclosed,
            } => {
                col = col.child(super::markdown_code_card::render_code_card(
                    &params,
                    idx,
                    lang.as_deref(),
                    code,
                    *unclosed,
                ));
            }
        }
    }
    col.into_any_element()
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
