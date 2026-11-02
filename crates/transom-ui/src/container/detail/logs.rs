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
    let toolbar = h_flex()
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
        );
    let body = if logs.is_empty() {
        centered_hint(rust_i18n::t!("container.logs.empty").to_string(), p)
    } else {
        // 逐行渲染而不是整块文本：反转顺序时才能保持每行独立可选。
        let rows = v_flex().gap_0().children(
            log_lines(logs, reversed)
                .into_iter()
                .map(|line| div().child(line.to_string())),
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
