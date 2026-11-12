//! 日志页签：跟随、行数与时间戳切换。

use super::*;
use crate::container::ContainerWorkspace;
use crate::widgets::status_dot_bare;
use gpui_kit::component::Side;
use gpui_kit::component::menu::PopupMenuItem;
use gpui_kit::prelude::FluentBuilder;
use transom_ui_icons::AppIcon;
use transom_ui_icons::AppIconElement;
use transom_ui_icons::UiIcon;

/// 日志页签渲染参数。
pub(crate) struct ContainerLogsView<'a> {
    pub(super) logs: &'a str,
    pub(super) follow: bool,
    pub(super) tail: usize,
    pub(super) timestamps: bool,
    pub(super) reversed: bool,
    pub(super) search: &'a Entity<InputState>,
    pub(super) filter: &'a str,
}

/// 根据容器名称生成稳定的彩色标签色（背景透明度与前景色）。
fn container_badge_color(name: &str) -> (gpui::Hsla, gpui::Hsla) {
    let mut hash: u32 = 5381;
    for b in name.bytes() {
        hash = hash.wrapping_mul(33).wrapping_add(b as u32);
    }
    let hues = [0.58, 0.38, 0.12, 0.78, 0.05, 0.48];
    let h = hues[(hash as usize) % hues.len()];
    let fg = gpui::hsla(h, 0.75, 0.65, 1.0);
    let bg = gpui::hsla(h, 0.75, 0.65, 0.18);
    (bg, fg)
}

/// 渲染单行日志：等宽 12px、行高 1.5，提取时间戳弱色，ERROR/WARN 语义着色，搜索关键词背景高亮。
fn render_log_line(line: &str, filter: &str, p: &Palette) -> AnyElement {
    let mut prefix_badge = None;
    let mut remaining_line = line;

    // 1. 解析多容器前缀，如 `[web-1] `
    if line.starts_with('[')
        && let Some(close_idx) = line.find(']')
    {
        let tag = &line[1..close_idx];
        if !tag.is_empty() && tag != "stderr" {
            let (bg, fg) = container_badge_color(tag);
            prefix_badge = Some(
                div()
                    .mr_1p5()
                    .px_1()
                    .py_0p5()
                    .rounded_xs()
                    .bg(bg)
                    .text_color(fg)
                    .font_weight(FontWeight::BOLD)
                    .child(format!("[{tag}]")),
            );
            remaining_line = line[close_idx + 1..].trim_start();
        }
    }

    // 2. 尝试提取时间戳弱色显示（标准 RFC3339 类似 2026-11-05T09:41:25.123456789Z）
    let mut timestamp_prefix = None;
    if remaining_line.len() >= 20 {
        let candidate = &remaining_line[..20];
        if candidate.as_bytes()[4] == b'-'
            && candidate.as_bytes()[7] == b'-'
            && (candidate.as_bytes()[10] == b'T' || candidate.as_bytes()[10] == b' ')
            && let Some(space_idx) = remaining_line.find(' ')
        {
            let ts = &remaining_line[..space_idx];
            timestamp_prefix = Some(
                div()
                    .mr_2()
                    .text_size(rems(0.6875))
                    .text_color(p.muted_foreground)
                    .child(ts.to_string()),
            );
            remaining_line = remaining_line[space_idx + 1..].trim_start();
        }
    }

    // 3. 语义着色：ERROR / WARN
    let upper = remaining_line.to_uppercase();
    let text_color = if upper.contains("ERROR") || upper.contains("FATAL") || upper.contains("FAIL")
    {
        p.danger
    } else if upper.contains("WARN") {
        p.warning
    } else {
        p.foreground
    };

    // 4. 搜索高亮
    let filter = filter.trim();
    let text_element: AnyElement = if filter.is_empty() {
        div()
            .text_color(text_color)
            .child(remaining_line.to_string())
            .into_any_element()
    } else {
        let filter_lower = filter.to_lowercase();
        let line_lower = remaining_line.to_lowercase();
        let mut spans = Vec::new();
        let mut last_end = 0;
        for (start, _) in line_lower.match_indices(&filter_lower) {
            if start > last_end {
                spans.push(
                    div()
                        .text_color(text_color)
                        .child(remaining_line[last_end..start].to_string()),
                );
            }
            let end = start + filter.len();
            spans.push(
                div()
                    .bg(p.warning.opacity(0.3))
                    .text_color(p.foreground)
                    .rounded_xs()
                    .child(remaining_line[start..end].to_string()),
            );
            last_end = end;
        }
        if last_end < remaining_line.len() {
            spans.push(
                div()
                    .text_color(text_color)
                    .child(remaining_line[last_end..].to_string()),
            );
        }
        h_flex().gap_0().children(spans).into_any_element()
    };

    let mut line_row = h_flex()
        .w_full()
        .items_baseline()
        .font_family(p.mono_font_family.clone())
        .text_size(rems(0.75))
        .line_height(relative(1.5));

    if let Some(badge) = prefix_badge {
        line_row = line_row.child(badge);
    }
    if let Some(ts) = timestamp_prefix {
        line_row = line_row.child(ts);
    }
    line_row.child(text_element).into_any_element()
}

pub(crate) fn render_logs(
    view: &Entity<ContainerWorkspace>,
    params: ContainerLogsView<'_>,
    p: &Palette,
) -> AnyElement {
    let ContainerLogsView {
        logs,
        follow,
        tail,
        timestamps,
        reversed,
        search,
        filter,
    } = params;

    let all_lines_count = logs.lines().count();
    let lines = filtered_log_lines(logs, filter, reversed, LOG_RENDER_LIMIT);
    let matched_count = lines.len();

    // 1. 跟随主行动按钮
    let follow_btn = Button::new("container-logs-follow")
        .xsmall()
        .when(follow, |b| b.primary())
        .when(!follow, |b| b.ghost())
        .child(AppIconElement::new(AppIcon::Ui(UiIcon::DIFF)).small())
        .label(rust_i18n::t!("container.logs.follow").to_string())
        .on_click({
            let view = view.clone();
            move |_, _, cx| {
                view.update(cx, |workspace, cx| {
                    workspace.toggle_container_logs_follow(!follow);
                    cx.notify();
                });
            }
        });

    // 2. 尾部行数下拉菜单
    let tail_text = if tail == 0 {
        rust_i18n::t!("container.logs.tail_all").to_string()
    } else {
        rust_i18n::t!("container.logs.tail_format", count = tail).to_string()
    };
    let tail_view = view.clone();
    let tail_dropdown = Button::new("container-logs-tail-dropdown")
        .ghost()
        .xsmall()
        .label(tail_text)
        .child(AppIconElement::new(AppIcon::Ui(UiIcon::CHEVRON_DOWN)).small())
        .dropdown_menu(move |menu, _, _| {
            let mut menu = menu;
            for choice in LOG_TAIL_CHOICES {
                let label = if choice == 0 {
                    rust_i18n::t!("container.logs.tail_all").to_string()
                } else {
                    rust_i18n::t!("container.logs.tail_format", count = choice).to_string()
                };
                menu = menu.item(PopupMenuItem::new(label).checked(choice == tail).on_click({
                    let view = tail_view.clone();
                    move |_, _, cx| {
                        view.update(cx, |workspace, cx| {
                            workspace.set_container_logs_tail(choice);
                            cx.notify();
                        });
                    }
                }));
            }
            menu
        });

    // 3. 更多操作下拉菜单（时间戳、倒序、复制、导出、清空）
    let more_view = view.clone();
    let more_btn = Button::new("container-logs-more-menu")
        .ghost()
        .xsmall()
        .child(AppIconElement::new(AppIcon::Ui(UiIcon::MORE_HORIZONTAL)).small())
        .accessibility_label(rust_i18n::t!("common.more").to_string())
        .tooltip(rust_i18n::t!("common.more").to_string())
        .dropdown_menu(move |menu, _, _| {
            let timestamps_view = more_view.clone();
            let reverse_view = more_view.clone();
            let copy_view = more_view.clone();
            let export_view = more_view.clone();
            let clear_view = more_view.clone();

            let menu = menu.check_side(Side::Right);
            menu.item(
                PopupMenuItem::new(rust_i18n::t!("container.logs.timestamps").to_string())
                    .checked(timestamps)
                    .on_click(move |_, _, cx| {
                        timestamps_view.update(cx, |workspace, cx| {
                            workspace.toggle_container_logs_timestamps();
                            cx.notify();
                        });
                    }),
            )
            .item(
                PopupMenuItem::new(rust_i18n::t!("container.logs.reverse").to_string())
                    .checked(reversed)
                    .on_click(move |_, _, cx| {
                        reverse_view.update(cx, |workspace, cx| {
                            workspace.state.logs_reversed = !workspace.state.logs_reversed;
                            cx.notify();
                        });
                    }),
            )
            .separator()
            .item(crate::widgets::menu_item(
                rust_i18n::t!("container.logs.copy").to_string(),
                AppIcon::Ui(UiIcon::COPY),
                move |_, _, cx| {
                    copy_view.update(cx, |workspace, cx| {
                        workspace.copy_container_logs(cx);
                    });
                },
            ))
            .item(crate::widgets::menu_item(
                rust_i18n::t!("container.logs.export").to_string(),
                AppIcon::Ui(UiIcon::DOWNLOAD),
                move |_, _window, cx| {
                    export_view.update(cx, |workspace, cx| {
                        workspace.export_container_logs(cx);
                    });
                },
            ))
            .separator()
            .item(crate::widgets::menu_item(
                rust_i18n::t!("container.logs.clear").to_string(),
                AppIcon::Ui(UiIcon::DELETE),
                move |_, _, cx| {
                    clear_view.update(cx, |workspace, cx| {
                        workspace.state.logs.clear();
                        cx.notify();
                    });
                },
            ))
        });

    let toolbar = h_flex()
        .w_full()
        .px_3()
        .py_2()
        .items_center()
        .justify_between()
        .border_color(p.divider(p.border))
        .divider_edge(DividerEdge::Bottom, p.divider_width)
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .child(follow_btn)
                .child(tail_dropdown)
                .child(div().w(px(180.0)).child(Input::new(search).small())),
        )
        .child(more_btn);

    // 4. 底部弱色状态栏
    let status_text = if filter.trim().is_empty() {
        rust_i18n::t!("container.logs.status_total", count = all_lines_count).to_string()
    } else {
        rust_i18n::t!(
            "container.logs.status_matched",
            matched = matched_count,
            total = all_lines_count
        )
        .to_string()
    };

    let follow_status_text = if follow {
        rust_i18n::t!("container.logs.status_following").to_string()
    } else {
        rust_i18n::t!("container.logs.status_paused").to_string()
    };

    let status_bar = h_flex()
        .w_full()
        .px_3()
        .py_1()
        .items_center()
        .justify_between()
        .border_color(p.divider(p.border))
        .divider_edge(DividerEdge::Top, p.divider_width)
        .text_xs()
        .text_color(p.muted_foreground)
        .child(div().child(status_text))
        .child(
            h_flex()
                .gap_1p5()
                .items_center()
                .child(status_dot_bare(if follow {
                    p.success
                } else {
                    p.muted_foreground
                }))
                .child(div().child(follow_status_text)),
        );

    let body = if logs.is_empty() {
        centered_hint(rust_i18n::t!("container.logs.empty").to_string(), p)
    } else {
        let rows = v_flex().gap_0().children(
            lines
                .into_iter()
                .map(|line| render_log_line(line, filter, p)),
        );
        div()
            .id("container-logs-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px_3()
            .py_2()
            .font_family(p.mono_font_family.clone())
            .text_size(rems(0.75))
            .line_height(relative(1.5))
            .child(rows)
            .into_any_element()
    };

    v_flex()
        .size_full()
        .overflow_hidden()
        .child(toolbar)
        .child(body)
        .child(status_bar)
        .into_any_element()
}
