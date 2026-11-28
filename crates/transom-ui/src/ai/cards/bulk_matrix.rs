//! 多主机批量执行矩阵对比视图。
//!
//! 设计原理（Why）：
//! - `remote_exec_bulk` 返回的多主机长文本在单框中极难横向对比；
//! - 本模块将规范化的 `=== [host (login) | exit=N | Mms] ===` 输出拆解为结构化行，
//!   以第一台成功主机（或首台主机）为基准自动标注“一致 / 差异”，并支持按行点击展开详情。

use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::*;
use transom_ui_icons::{AppIcon, AppIconElement, UiIcon};

use crate::palette::Palette;
use crate::theme::tokens::{space, typography};

/// 单台主机的批量执行结果行。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app::ai) struct BulkHostRow {
    pub(in crate::app::ai) host_label: String,
    pub(in crate::app::ai) exit_code: Option<i32>,
    pub(in crate::app::ai) duration_ms: Option<u64>,
    pub(in crate::app::ai) body: String,
    pub(in crate::app::ai) differs_from_baseline: bool,
}

/// 解析 `remote_exec_bulk` 的聚合输出为结构化主机行列表。
pub(in crate::app::ai) fn parse_bulk_matrix(raw: &str) -> Vec<BulkHostRow> {
    let mut rows = Vec::new();
    let mut current_header: Option<String> = None;
    let mut current_body = Vec::new();

    for line in raw.lines() {
        let trimmed = line.trim();
        if let Some(inner) = trimmed
            .strip_prefix("=== [")
            .and_then(|s| s.strip_suffix("] ==="))
        {
            if let Some(hdr) = current_header.take() {
                rows.push(build_row(&hdr, &current_body.join("\n")));
                current_body.clear();
            }
            current_header = Some(inner.to_owned());
        } else if current_header.is_some() {
            current_body.push(line);
        }
    }
    if let Some(hdr) = current_header.take() {
        rows.push(build_row(&hdr, &current_body.join("\n")));
    }

    if rows.len() > 1 {
        let baseline = rows
            .iter()
            .find(|r| r.exit_code == Some(0))
            .or_else(|| rows.first())
            .map(|r| normalize_for_diff(&r.body))
            .unwrap_or_default();
        for row in &mut rows {
            row.differs_from_baseline = normalize_for_diff(&row.body) != baseline;
        }
    }
    rows
}

fn normalize_for_diff(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn build_row(header: &str, body: &str) -> BulkHostRow {
    let parts: Vec<&str> = header.split('|').map(str::trim).collect();
    let host_label = parts.first().copied().unwrap_or(header).to_owned();
    let mut exit_code = None;
    let mut duration_ms = None;
    for part in parts.iter().skip(1) {
        if let Some(code_str) = part.strip_prefix("exit=") {
            exit_code = code_str.trim().parse::<i32>().ok();
        } else if let Some(ms_str) = part.strip_suffix("ms") {
            duration_ms = ms_str.trim().parse::<u64>().ok();
        }
    }
    BulkHostRow {
        host_label,
        exit_code,
        duration_ms,
        body: body.trim().to_owned(),
        differs_from_baseline: false,
    }
}

/// 渲染多主机批量执行矩阵卡片内容。
pub(in crate::app::ai) fn render_bulk_matrix(
    card_id: &str,
    rows: &[BulkHostRow],
    expanded: bool,
    p: &Palette,
) -> AnyElement {
    let ok_count = rows.iter().filter(|r| r.exit_code == Some(0)).count();
    let diff_count = rows.iter().filter(|r| r.differs_from_baseline).count();

    let mut container = v_flex().w_full().gap(px(4.0)).child(
        h_flex()
            .w_full()
            .justify_between()
            .items_center()
            .px(px(4.0))
            .child(
                div()
                    .text_size(rems(typography::CAPTION))
                    .text_color(p.text.muted)
                    .child(
                        rust_i18n::t!("ai.bulk_matrix_summary", ok = ok_count, total = rows.len())
                            .to_string(),
                    ),
            )
            .child(
                div()
                    .text_size(rems(typography::CAPTION))
                    .text_color(if diff_count > 0 { p.warning } else { p.success })
                    .child(if diff_count > 0 {
                        rust_i18n::t!("ai.bulk_matrix_diff_count", count = diff_count).to_string()
                    } else {
                        rust_i18n::t!("ai.bulk_matrix_all_same").to_string()
                    }),
            ),
    );

    for (idx, row) in rows.iter().enumerate() {
        let is_ok = row.exit_code == Some(0);
        let status_color = if is_ok { p.success } else { p.danger };
        let status_icon = if is_ok { UiIcon::CHECK } else { UiIcon::CLOSE };
        let exit_text = row
            .exit_code
            .map(|c| format!("exit {c}"))
            .unwrap_or_else(|| "exit ?".to_owned());
        let ms_text = row
            .duration_ms
            .map(|ms| format!("{ms}ms"))
            .unwrap_or_default();

        let mut row_box = v_flex()
            .id((SharedString::from(format!("bulk-row-{card_id}")), idx))
            .w_full()
            .px(rems(space::XS))
            .py(px(4.0))
            .gap(px(3.0))
            .rounded(p.radius.sm)
            .bg(p.background)
            .border_1()
            .border_color(if row.differs_from_baseline {
                p.warning.opacity(0.6)
            } else {
                p.divider(p.border)
            })
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .items_center()
                    .gap(px(6.0))
                    .child(
                        h_flex()
                            .flex_1()
                            .gap(px(6.0))
                            .items_center()
                            .overflow_hidden()
                            .child(
                                div()
                                    .text_color(status_color)
                                    .child(AppIconElement::new(AppIcon::Ui(status_icon)).small()),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .truncate()
                                    .text_size(rems(typography::CAPTION))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(p.foreground)
                                    .child(row.host_label.clone()),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap(px(6.0))
                            .items_center()
                            .child(if row.differs_from_baseline {
                                div()
                                    .px(px(5.0))
                                    .py(px(1.0))
                                    .rounded(p.radius.sm)
                                    .bg(p.warning.opacity(0.15))
                                    .text_size(rems(typography::CAPTION))
                                    .text_color(p.warning)
                                    .child(rust_i18n::t!("ai.bulk_matrix_diff").to_string())
                            } else {
                                div()
                            })
                            .child(
                                div()
                                    .text_size(rems(typography::CAPTION))
                                    .text_color(status_color)
                                    .child(exit_text),
                            )
                            .child(
                                div()
                                    .text_size(rems(typography::CAPTION))
                                    .text_color(p.text.muted)
                                    .child(ms_text),
                            ),
                    ),
            );

        if expanded && !row.body.is_empty() {
            row_box = row_box.child(
                div()
                    .id((SharedString::from(format!("bulk-body-{card_id}")), idx))
                    .max_h(rems(8.0))
                    .overflow_y_scrollbar()
                    .pt(px(2.0))
                    .font_family("monospace")
                    .text_size(rems(typography::CAPTION))
                    .text_color(p.muted_foreground)
                    .child(row.body.clone()),
            );
        }
        container = container.child(row_box);
    }

    container.into_any_element()
}

#[cfg(test)]
mod tests {
    use super::parse_bulk_matrix;

    #[test]
    fn parses_host_rows_and_marks_divergence() {
        let raw = "=== [web-01 (root@10.0.0.1) | exit=0 | 42ms] ===\nactive\n\n=== [web-02 (root@10.0.0.2) | exit=3 | 55ms] ===\ninactive";
        let rows = parse_bulk_matrix(raw);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].exit_code, Some(0));
        assert_eq!(rows[0].duration_ms, Some(42));
        assert!(!rows[0].differs_from_baseline);
        assert!(rows[1].differs_from_baseline);
    }
}
