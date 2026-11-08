//! 容器进程可折叠表格卡片。

use super::*;
use crate::widgets::table_header;
use gpui_kit::component::Disableable;

pub(crate) struct ContainerProcessesView<'a> {
    pub(crate) tab_id: TabId,
    pub(crate) processes: &'a [ContainerProcess],
    pub(crate) loading: bool,
    pub(crate) error: Option<&'a str>,
    pub(crate) is_running: bool,
    pub(crate) collapsed: bool,
}

pub(crate) fn processes_card(
    view: ContainerProcessesView<'_>,
    chrome: &StatsChrome,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let ContainerProcessesView {
        tab_id,
        processes,
        loading,
        error,
        is_running,
        collapsed,
    } = view;
    let p = chrome.p;
    let refresh_tooltip = if is_running {
        rust_i18n::t!("container.stats.process_refresh").to_string()
    } else {
        rust_i18n::t!("container.stats.not_running_short").to_string()
    };

    let summary = h_flex()
        .gap_2()
        .items_center()
        .child(
            div()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(if loading {
                    rust_i18n::t!("container.stats.process_loading").to_string()
                } else if !is_running {
                    "-".to_string()
                } else {
                    processes.len().to_string()
                }),
        )
        .child(
            Button::new("container-processes-refresh")
                .ghost()
                .xsmall()
                .icon(IconName::RefreshCw)
                .disabled(!is_running)
                .tooltip(refresh_tooltip)
                .on_click(cx.listener(move |this, _, _, cx| {
                    if is_running {
                        this.ensure_container_processes(tab_id, cx);
                        cx.notify();
                    }
                })),
        );

    let header_btn = h_flex()
        .gap_2()
        .items_center()
        .justify_between()
        .w_full()
        .px_3()
        .py_2p5()
        .child(
            h_flex()
                .id("btn-toggle-stats-processes")
                .gap_1p5()
                .items_center()
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.toggle_container_stats_processes(tab_id, cx);
                }))
                .child(
                    Icon::new(if collapsed {
                        IconName::ChevronRight
                    } else {
                        IconName::ChevronDown
                    })
                    .size(px(14.0))
                    .text_color(p.muted_foreground),
                )
                .child(
                    Icon::new(IconName::ListTree)
                        .size(px(14.0))
                        .text_color(p.muted_foreground),
                )
                .child(
                    div()
                        .text_size(px(11.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(p.muted_foreground)
                        .child(rust_i18n::t!("container.stats.processes").to_string()),
                ),
        )
        .child(summary);

    let mut card = v_flex()
        .id("container-stats-card-processes")
        .w_full()
        .rounded(px(10.0))
        .bg(p.muted.opacity(0.04))
        .overflow_hidden()
        .child(header_btn);

    if !collapsed {
        let body: AnyElement = if let Some(error) = error {
            div()
                .px_3()
                .pb_3()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(error.to_string())
                .into_any_element()
        } else if !is_running {
            div()
                .px_3()
                .pb_3()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("container.stats.process_not_running").to_string())
                .into_any_element()
        } else if processes.is_empty() {
            div()
                .px_3()
                .pb_3()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("container.stats.process_empty").to_string())
                .into_any_element()
        } else {
            let mut sorted: Vec<&ContainerProcess> = processes.iter().collect();
            sorted.sort_by(|left, right| {
                right
                    .cpu_percent
                    .partial_cmp(&left.cpu_percent)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });

            let th = table_header(
                vec![
                    div()
                        .w(px(64.0))
                        .flex_none()
                        .child("PID".to_string())
                        .into_any_element(),
                    div()
                        .w(px(80.0))
                        .flex_none()
                        .child(rust_i18n::t!("container.stats.process_user").to_string())
                        .into_any_element(),
                    div()
                        .w(px(64.0))
                        .flex_none()
                        .text_right()
                        .child("CPU".to_string())
                        .into_any_element(),
                    div()
                        .w(px(64.0))
                        .flex_none()
                        .text_right()
                        .child(rust_i18n::t!("container.stats.process_mem").to_string())
                        .into_any_element(),
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(rust_i18n::t!("container.stats.process_command").to_string())
                        .into_any_element(),
                ],
                p,
            );

            let mut table = v_flex().w_full().child(th).children(
                sorted
                    .iter()
                    .take(PROCESS_LIMIT)
                    .map(|process| process_row(process, p)),
            );
            if sorted.len() > PROCESS_LIMIT {
                table = table.child(
                    div()
                        .px_3()
                        .pt_1()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(
                            rust_i18n::t!("container.stats.process_limit", count = PROCESS_LIMIT)
                                .to_string(),
                        ),
                );
            }
            table.pb_2().into_any_element()
        };

        card = card.child(body);
    }

    card.into_any_element()
}

pub(crate) fn process_row(process: &ContainerProcess, p: &Palette) -> AnyElement {
    h_flex()
        .w_full()
        .px_3()
        .py_1p5()
        .gap_2()
        .items_center()
        .text_xs()
        .child(
            div()
                .w(px(64.0))
                .flex_none()
                .font_family(p.mono_font_family.clone())
                .text_color(p.muted_foreground)
                .child(process.pid.clone()),
        )
        .child(
            div()
                .w(px(80.0))
                .flex_none()
                .truncate()
                .text_color(p.foreground)
                .child(process.user.clone()),
        )
        .child(
            div()
                .w(px(64.0))
                .flex_none()
                .text_right()
                .font_family(p.mono_font_family.clone())
                .text_color(usage_color(process.cpu_percent, p))
                .child(format!("{:.1}%", process.cpu_percent)),
        )
        .child(
            div()
                .w(px(64.0))
                .flex_none()
                .text_right()
                .font_family(p.mono_font_family.clone())
                .text_color(p.muted_foreground)
                .child(format!("{:.1}%", process.mem_percent)),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .font_family(p.mono_font_family.clone())
                .text_color(p.foreground)
                .child(process.command.clone()),
        )
        .into_any_element()
}
