//! 日志页签：跟随、行数与时间戳切换。

use super::*;

/// 日志页签渲染参数。
pub(crate) struct ContainerLogsView<'a> {
    pub(super) tab_id: TabId,
    pub(super) logs: &'a str,
    pub(super) follow: bool,
    pub(super) tail: usize,
    pub(super) timestamps: bool,
    pub(super) reversed: bool,
    pub(super) search: &'a Entity<InputState>,
    pub(super) filter: &'a str,
}

/// 渲染单行日志，并在存在搜索关键字时对命中部分进行背景高亮。
fn render_log_line(line: &str, filter: &str, p: &Palette) -> AnyElement {
    let filter = filter.trim();
    if filter.is_empty() {
        return div().child(line.to_string()).into_any_element();
    }
    let filter_lower = filter.to_lowercase();
    let line_lower = line.to_lowercase();
    let mut spans = Vec::new();
    let mut last_end = 0;
    for (start, _) in line_lower.match_indices(&filter_lower) {
        if start > last_end {
            spans.push(div().child(line[last_end..start].to_string()));
        }
        let end = start + filter.len();
        spans.push(
            div()
                .bg(p.sidebar_accent.opacity(0.55))
                .rounded_xs()
                .child(line[start..end].to_string()),
        );
        last_end = end;
    }
    if last_end < line.len() {
        spans.push(div().child(line[last_end..].to_string()));
    }
    h_flex().gap_0().children(spans).into_any_element()
}

pub(crate) fn render_logs(
    view: ContainerLogsView<'_>,
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let ContainerLogsView {
        tab_id,
        logs,
        follow,
        tail,
        timestamps,
        reversed,
        search,
        filter,
    } = view;
    let mut tails = h_flex().gap_0p5().items_center();
    for choice in LOG_TAIL_CHOICES {
        tails = tails.child(
            Button::new(ElementId::Name(
                format!("container-logs-tail-{choice}").into(),
            ))
            .ghost()
            .xsmall()
            .selected(choice == tail)
            .label(if choice == 0 {
                rust_i18n::t!("container.logs.tail_all").to_string()
            } else {
                choice.to_string()
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                this.set_container_logs_tail(tab_id, choice);
                cx.notify();
            })),
        );
    }

    let all_lines_count = logs.lines().count();
    let lines = filtered_log_lines(logs, filter, reversed);
    let matched_count = lines.len();

    let mut toolbar = h_flex()
        .gap_2()
        .p_2()
        .items_center()
        // 工具条与日志正文之间要有分隔线：两者背景同色，不留线时正文像浮在半空。
        .border_color(p.divider(p.border))
        .divider_edge(DividerEdge::Bottom, p.divider_width)
        .child(tails)
        .child(
            Button::new("container-logs-timestamps")
                .ghost()
                .xsmall()
                .icon(IconName::Clock)
                .label(rust_i18n::t!("container.logs.timestamps").to_string())
                .selected(timestamps)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.toggle_container_logs_timestamps(tab_id);
                    cx.notify();
                })),
        )
        .child(
            Button::new("container-logs-reverse")
                .ghost()
                .xsmall()
                .icon(IconName::ArrowDown)
                .label(rust_i18n::t!("container.logs.reverse").to_string())
                .selected(reversed)
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(workspace) = this.state.container_workspace_mut(tab_id) {
                        workspace.logs_reversed = !workspace.logs_reversed;
                    }
                    cx.notify();
                })),
        )
        .child(
            Button::new("container-logs-copy")
                .ghost()
                .xsmall()
                .icon(IconName::Copy)
                .label(rust_i18n::t!("container.logs.copy").to_string())
                .on_click(cx.listener(move |this, _, _window, cx| {
                    this.copy_container_logs(tab_id, cx);
                })),
        )
        .child(
            Button::new("container-logs-export")
                .ghost()
                .xsmall()
                .icon(IconName::Download)
                .label(rust_i18n::t!("container.logs.export").to_string())
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.export_container_logs(tab_id, window, cx);
                })),
        )
        .child(
            Button::new("container-logs-follow")
                .xsmall()
                .icon(IconName::ScrollText)
                .label(rust_i18n::t!("container.logs.follow").to_string())
                .selected(follow)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.toggle_container_logs_follow(tab_id, !follow);
                    cx.notify();
                })),
        )
        .child(
            Button::new("container-logs-clear")
                .ghost()
                .xsmall()
                .icon(IconName::Trash)
                .label(rust_i18n::t!("container.logs.clear").to_string())
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(workspace) = this.state.container_workspace_mut(tab_id) {
                        workspace.logs.clear();
                    }
                    cx.notify();
                })),
        )
        .child(
            div()
                .w(px(160.0))
                .child(Input::new(search).small()),
        );

    if !filter.trim().is_empty() {
        toolbar = toolbar.child(
            div()
                .text_xs()
                .text_color(p.muted)
                .child(rust_i18n::t!(
                    "container.logs.matched_lines",
                    matched = matched_count.to_string(),
                    total = all_lines_count.to_string()
                )),
        );
    }

    let body = if logs.is_empty() {
        centered_hint(rust_i18n::t!("container.logs.empty").to_string(), p)
    } else {
        // 逐行渲染而不是整块文本：反转顺序与高亮时保持每行独立。
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
            .p_2()
            .font_family(p.mono_font_family.clone())
            .text_size(p.mono_font_size)
            .line_height(relative(1.35))
            .child(rows)
            .into_any_element()
    };
    v_flex()
        .size_full()
        .overflow_hidden()
        .child(toolbar)
        .child(body)
        .into_any_element()
}
