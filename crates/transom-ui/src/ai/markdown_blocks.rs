//! AI 消息流 Markdown 分段解析与代码/命令块卡片渲染。
//!
//! 设计原理（Why）：
//! - `TextView::markdown` 在流式未闭合围栏或纯代码块场景下缺少复制、填入终端与语言标签交互；
//! - 先将 Markdown 按围栏（```` ``` ```` / `~~~`）切分为 [`MarkdownSegment::Prose`] 与
//!   [`MarkdownSegment::CodeBlock`]，普通正文继续交给 `TextView::markdown` 渲染（保留标题/列表/表格/选中），
//!   代码块则渲染为带语言图标、一键复制、Shell 填入终端与存为快捷指令的独立卡片。

use std::rc::Rc;

use gpui_kit::base::{TextView, TextViewStyle, h_flex, v_flex};
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::*;
use transom_ui_icons::{AppIcon, AppIconElement, CustomIcon, DomainIcon, UiIcon};

use crate::palette::Palette;
use crate::theme::tokens::{space, typography};

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
/// 收窄标题字号梯度、为表格提供圆角与宽松内边距、柔化分割线与行内代码胶囊背景。
fn build_markdown_style(p: &Palette) -> TextViewStyle {
    let mut table = StyleRefinement::default();
    table.overflow.x = Some(Overflow::Scroll);
    table = table.rounded(p.radius.md).border_color(p.stroke.subtle);

    let table_head = StyleRefinement::default()
        .bg(p.surface.faint)
        .text_color(p.text.subtle)
        .font_weight(FontWeight::MEDIUM);

    let table_cell = StyleRefinement::default()
        .px(px(12.0))
        .py(px(8.0))
        .border_color(p.stroke.faint);

    let blockquote_style = StyleRefinement::default()
        .border_color(p.stroke.muted)
        .text_color(p.text.subtle);

    TextViewStyle::default()
        .with_foreground(p.foreground)
        .with_muted_foreground(p.muted_foreground)
        .with_link(p.primary)
        .with_border(p.stroke.faint)
        .with_code_background(p.surface.faint)
        .with_dark(p.background.l < 0.5)
        .with_paragraph_gap(rems(0.65))
        .with_heading(|level| {
            let (size, weight) = match level {
                1 => (px(16.5), FontWeight::SEMIBOLD),
                2 => (px(15.0), FontWeight::SEMIBOLD),
                3 => (px(14.0), FontWeight::MEDIUM),
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
                col = col.child(
                    div().w_full().line_height(relative(1.58)).child(
                        TextView::markdown(
                            SharedString::from(format!("ai-md-{}-{idx}", params.msg_id)),
                            prose.clone(),
                        )
                        .style(md_style.clone())
                        .selectable(true),
                    ),
                );
            }
            MarkdownSegment::CodeBlock {
                lang,
                code,
                unclosed,
            } => {
                col = col.child(render_code_card(
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

fn render_code_card(
    params: &MarkdownRenderParams<'_>,
    idx: usize,
    lang: Option<&str>,
    code: &str,
    unclosed: bool,
) -> AnyElement {
    let p = params.p;
    let block_key = format!("code-{}-{idx}", params.msg_id);
    let is_copied = params.copied_key == Some(block_key.as_str());
    let is_shell = is_shell_language(lang);
    let lang_label = lang
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| rust_i18n::t!("ai.code_block_default").to_string());

    let icon = if is_shell {
        AppIcon::Domain(DomainIcon::TERMINAL)
    } else {
        AppIcon::Custom(CustomIcon::CPU)
    };

    let mut left = h_flex().gap(px(6.0)).items_center().child(
        h_flex()
            .px(px(6.0))
            .py(px(2.0))
            .gap(px(5.0))
            .items_center()
            .rounded(p.radius.sm)
            .bg(p.surface.floating)
            .border_1()
            .border_color(p.stroke.faint)
            .child(
                div()
                    .text_color(p.text.muted)
                    .child(AppIconElement::new(icon).small()),
            )
            .child(
                div()
                    .font_family("monospace")
                    .text_size(rems(typography::CAPTION))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(p.foreground)
                    .child(lang_label),
            ),
    );
    if unclosed {
        left = left.child(
            div()
                .px(px(6.0))
                .py(px(1.0))
                .rounded_full()
                .bg(p.tone.primary_subtle)
                .text_size(rems(typography::CAPTION))
                .font_weight(FontWeight::MEDIUM)
                .text_color(p.primary)
                .child(rust_i18n::t!("ai.streaming_code").to_string()),
        );
    }

    let copy_icon = if is_copied {
        UiIcon::CHECK
    } else {
        UiIcon::COPY
    };
    let copy_tip = if is_copied {
        rust_i18n::t!("ai.copied").to_string()
    } else {
        rust_i18n::t!("ai.copy").to_string()
    };
    let on_copy = Rc::clone(&params.callbacks.on_copy);
    let copy_code = code.to_owned();
    let copy_key = block_key.clone();

    let mut actions = h_flex().gap(px(4.0)).items_center();
    if is_shell && !unclosed {
        if params.has_active_terminal {
            let on_send = Rc::clone(&params.callbacks.on_send_to_terminal);
            let send_code = code.to_owned();
            actions = actions.child(
                Button::new(SharedString::from(format!("ai-send-term-{block_key}")))
                    .ghost()
                    .xsmall()
                    .label(rust_i18n::t!("ai.send_to_terminal").to_string())
                    .on_click(move |_, w, cx| on_send(send_code.clone(), w, cx)),
            );
        }
        let on_snip = Rc::clone(&params.callbacks.on_save_snippet);
        let snip_code = code.to_owned();
        let snip_name = code
            .lines()
            .next()
            .unwrap_or("")
            .trim()
            .chars()
            .take(32)
            .collect::<String>();
        actions = actions.child(
            Button::new(SharedString::from(format!("ai-snip-code-{block_key}")))
                .ghost()
                .xsmall()
                .label(rust_i18n::t!("ai.save_as_snippet").to_string())
                .on_click(move |_, w, cx| on_snip(snip_name.clone(), snip_code.clone(), w, cx)),
        );
    }
    actions = actions.child(
        Button::new(SharedString::from(format!("ai-copy-code-{block_key}")))
            .ghost()
            .xsmall()
            .accessibility_label(copy_tip.clone())
            .tooltip(copy_tip)
            .child(
                div()
                    .text_color(if is_copied { p.success } else { p.text.muted })
                    .child(AppIconElement::new(AppIcon::Ui(copy_icon)).small()),
            )
            .on_click(move |_, w, cx| on_copy(copy_key.clone(), copy_code.clone(), w, cx)),
    );

    let header = h_flex()
        .w_full()
        .px(rems(space::SM))
        .py(px(5.0))
        .justify_between()
        .items_center()
        .bg(p.surface.faint)
        .border_b_1()
        .border_color(p.stroke.subtle)
        .child(left)
        .child(actions);

    let line_count = code.lines().count().max(1);
    let line_numbers: String = (1..=line_count)
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let body_text = if code.is_empty() {
        " ".to_owned()
    } else {
        code.to_owned()
    };

    v_flex()
        .w_full()
        .rounded(p.radius.md)
        .overflow_hidden()
        .border_1()
        .border_color(if unclosed {
            p.primary.opacity(0.35)
        } else {
            p.stroke.subtle
        })
        .bg(p.background)
        .child(header)
        .child(
            h_flex()
                .w_full()
                .items_start()
                .child(
                    div()
                        .flex_shrink_0()
                        .px(rems(space::XS))
                        .py(rems(space::XS))
                        .border_r_1()
                        .border_color(p.stroke.faint)
                        .bg(p.surface.faint)
                        .font_family("monospace")
                        .text_size(rems(typography::CAPTION))
                        .line_height(relative(1.5))
                        .text_color(p.text.disabled)
                        .text_right()
                        .child(line_numbers),
                )
                .child(
                    div()
                        .id(SharedString::from(format!("ai-code-body-{block_key}")))
                        .flex_1()
                        .min_w_0()
                        .px(rems(space::SM))
                        .py(rems(space::XS))
                        .overflow_x_scroll()
                        .font_family("monospace")
                        .text_size(rems(typography::CAPTION))
                        .line_height(relative(1.5))
                        .text_color(p.foreground)
                        .child(body_text),
                ),
        )
        .into_any_element()
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
