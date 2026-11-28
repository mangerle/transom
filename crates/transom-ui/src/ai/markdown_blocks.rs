//! AI 消息流 Markdown 排版样式与混合分段渲染（纯解析状态机见 [`super::markdown_parser`]）。

use std::rc::Rc;
use std::time::Duration;

use gpui_kit::base::{TextView, TextViewMotion, TextViewStyle, v_flex};
use gpui_kit::*;

pub(super) use super::markdown_parser::{
    MarkdownSegment, StreamingMarkdownCache, is_shell_language, parse_markdown_segments,
};
use crate::palette::Palette;
use crate::theme::tokens::space;

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
