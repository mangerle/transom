//! Compose YAML 语法校验、路径面包屑与等宽行号槽支持。
//!
//! 设计原理（Why）：
//! - 将 YAML 行级解析与校验算法从通用视图渲染中剥离，形成可独立运行单元测试的内聚模块；
//! - 面包屑以层级分段直观显示项目名与配置文件相对层级，减少冗长绝对路径造成的视线遮挡；
//! - 行号槽与编辑器等高对齐，语法错误发生时将错误行号标红聚焦，极大提升开发者就地排障效率。

use crate::palette::Palette;
use crate::theme::tokens::{space, typography};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::input::TextareaState;
use gpui_kit::{AnyElement, FontWeight, IntoElement, ParentElement, Pixels, Styled, div, px, rems};

/// YAML 行级语法校验错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct YamlValidationError {
    pub(crate) line: usize,
    pub(crate) column: usize,
    pub(crate) message: String,
}

/// 对 Compose YAML 文本进行行级语法校验。
pub(crate) fn validate_compose_yaml(content: &str) -> Option<YamlValidationError> {
    if content.trim().is_empty() {
        return None;
    }
    match serde_yaml_ng::from_str::<serde_yaml_ng::Value>(content) {
        Ok(_) => None,
        Err(err) => {
            let (line, col) = err
                .location()
                .map(|loc| (loc.line(), loc.column()))
                .unwrap_or((1, 1));
            let first_line = err
                .to_string()
                .lines()
                .next()
                .unwrap_or("syntax error")
                .to_string();
            Some(YamlValidationError {
                line,
                column: col,
                message: first_line,
            })
        }
    }
}

/// 渲染文件路径面包屑导航。
pub(crate) fn render_path_breadcrumbs(
    project_name: &str,
    path: &std::path::Path,
    p: &Palette,
) -> AnyElement {
    let mut segments = Vec::new();
    segments.push(project_name.to_string());

    if let Some(file_name) = path.file_name().and_then(|f| f.to_str()) {
        if let Some(parent) = path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|f| f.to_str())
        {
            segments.push(parent.to_string());
        }
        segments.push(file_name.to_string());
    } else {
        segments.push(path.display().to_string());
    }

    let mut crumbs = h_flex().gap_1p5().items_center();
    for (i, seg) in segments.iter().enumerate() {
        if i > 0 {
            crumbs = crumbs.child(div().text_xs().text_color(p.text.disabled).child("/"));
        }
        let is_last = i == segments.len() - 1;
        crumbs = crumbs.child(
            div()
                .text_xs()
                .font_weight(if is_last {
                    FontWeight::BOLD
                } else {
                    FontWeight::NORMAL
                })
                .text_color(if is_last {
                    p.foreground
                } else {
                    p.muted_foreground
                })
                .child(seg.clone()),
        );
    }
    crumbs.into_any_element()
}

/// 行号槽与文本排版的同步度量。
///
/// `TextareaState` 没有公开 `EditorState::text_origin_offset()`；后者也只描述
/// 水平原点。这里改用文本区真实的 `line_height()`、`scroll_offset()` 和首行
/// 渲染边界推导垂直原点，首帧尚未布局时才回退到设计令牌。
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct LineNumberMetrics {
    line_height: Pixels,
    top_padding: Pixels,
    scroll_y: Pixels,
}

impl LineNumberMetrics {
    /// 首帧布局前的设计令牌回退值。
    pub(crate) fn fallback(rem_size: Pixels) -> Self {
        Self {
            line_height: rems(typography::BODY + space::COMPACT).to_pixels(rem_size),
            top_padding: rems(space::SM + space::COMPACT).to_pixels(rem_size),
            scroll_y: px(0.),
        }
    }
}

/// 从文本区最近一次布局读取行号槽所需度量。
pub(crate) fn line_number_metrics(editor: &TextareaState, rem_size: Pixels) -> LineNumberMetrics {
    let fallback = LineNumberMetrics::fallback(rem_size);
    let line_height = editor.line_height().unwrap_or(fallback.line_height);
    let scroll_y = editor.scroll_offset().y;
    let outer_padding = rems(space::SM).to_pixels(rem_size);
    let top_padding = editor
        .range_to_bounds(&(0..0))
        .map(|bounds| {
            (bounds.origin.y - editor.input_bounds().origin.y - scroll_y + outer_padding)
                .max(px(0.))
        })
        .unwrap_or(fallback.top_padding);
    LineNumberMetrics {
        line_height,
        top_padding,
        scroll_y,
    }
}

/// 渲染编辑器左侧等宽行号槽。
pub(crate) fn render_line_numbers(
    line_count: usize,
    error_line: Option<usize>,
    metrics: LineNumberMetrics,
    p: &Palette,
) -> AnyElement {
    let mut numbers = v_flex()
        .w_full()
        .pt(metrics.top_padding)
        .mt(metrics.scroll_y)
        .items_end();

    for line_num in 1..=line_count.max(1) {
        let is_err = error_line == Some(line_num);
        numbers = numbers.child(
            div()
                .h(metrics.line_height)
                .text_size(rems(typography::CAPTION))
                .font_family(p.mono_font_family.clone())
                .font_weight(if is_err {
                    FontWeight::BOLD
                } else {
                    FontWeight::NORMAL
                })
                .text_color(if is_err {
                    p.danger
                } else {
                    p.muted_foreground.opacity(0.5)
                })
                .child(format!("{line_num}")),
        );
    }

    v_flex()
        .w(px(38.0))
        .h_full()
        .pr_1p5()
        .items_end()
        .overflow_hidden()
        .bg(p.surface.faint)
        .border_r_1()
        .border_color(p.stroke.muted)
        .child(numbers)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_number_fallback_scales_with_rem_size() {
        let metrics = LineNumberMetrics::fallback(px(16.));
        assert_eq!(metrics.line_height, px(20.));
        assert_eq!(metrics.top_padding, px(14.));
        assert_eq!(metrics.scroll_y, px(0.));

        let scaled = LineNumberMetrics::fallback(px(20.));
        assert_eq!(scaled.line_height, px(25.));
        assert_eq!(scaled.top_padding, px(17.5));
    }

    #[test]
    fn test_validate_empty_yaml() {
        assert!(validate_compose_yaml("").is_none());
        assert!(validate_compose_yaml("   \n\t  ").is_none());
    }

    #[test]
    fn test_validate_valid_yaml() {
        let yaml = r#"
version: '3.8'
services:
  web:
    image: nginx:alpine
    ports:
      - "80:80"
"#;
        assert!(validate_compose_yaml(yaml).is_none());
    }

    #[test]
    fn test_validate_invalid_yaml_with_line() {
        let yaml = "version: '3.8'\nservices:\n  web:\n    image: [unclosed";
        let err = validate_compose_yaml(yaml);
        assert!(err.is_some());
        let err = err.unwrap();
        assert!(err.line >= 4);
    }
}
