//! Compose YAML 语法校验、路径面包屑与等宽行号槽支持。
//!
//! 设计原理（Why）：
//! - 将 YAML 行级解析与校验算法从通用视图渲染中剥离，形成可独立运行单元测试的内聚模块；
//! - 面包屑以层级分段直观显示项目名与配置文件相对层级，减少冗长绝对路径造成的视线遮挡；
//! - 行号槽与编辑器等高对齐，语法错误发生时将错误行号标红聚焦，极大提升开发者就地排障效率。

use crate::palette::Palette;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::{AnyElement, FontWeight, IntoElement, ParentElement, Styled, div, px};

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
        if let Some(parent) = path.parent().and_then(|p| p.file_name()).and_then(|f| f.to_str()) {
            segments.push(parent.to_string());
        }
        segments.push(file_name.to_string());
    } else {
        segments.push(path.display().to_string());
    }

    let mut crumbs = h_flex().gap_1p5().items_center();
    for (i, seg) in segments.iter().enumerate() {
        if i > 0 {
            crumbs = crumbs.child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground.opacity(0.4))
                    .child("/"),
            );
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

/// 渲染编辑器左侧等宽行号槽。
pub(crate) fn render_line_numbers(
    line_count: usize,
    error_line: Option<usize>,
    p: &Palette,
) -> AnyElement {
    let mut gutter = v_flex()
        .w(px(38.0))
        .h_full()
        .py_2()
        .pr_1p5()
        .items_end()
        .overflow_hidden()
        .bg(p.muted.opacity(0.08))
        .border_r_1()
        .border_color(p.border.opacity(0.3));

    for line_num in 1..=line_count.max(1) {
        let is_err = error_line == Some(line_num);
        gutter = gutter.child(
            div()
                .h(px(20.0))
                .text_size(px(11.0))
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

    gutter.into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

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
