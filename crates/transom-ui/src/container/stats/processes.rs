//! 容器进程列表。

use super::*;

pub(crate) fn processes_card(
    tab_id: TabId,
    processes: &[ContainerProcess],
    loading: bool,
    error: Option<&str>,
    chrome: &StatsChrome,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let p = chrome.p;
    let summary = h_flex()
        .gap_1p5()
        .items_center()
        .child(
            div()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(if loading {
                    rust_i18n::t!("container.stats.process_loading").to_string()
                } else {
                    processes.len().to_string()
                }),
        )
        .child(
            Button::new("container-processes-refresh")
                .ghost()
                .xsmall()
                .icon(IconName::RefreshCw)
                .tooltip(rust_i18n::t!("container.stats.process_refresh").to_string())
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.ensure_container_processes(tab_id);
                    cx.notify();
                })),
        )
        .into_any_element();
    let body: AnyElement = if let Some(error) = error {
        div()
            .text_xs()
            .text_color(p.muted_foreground)
            .child(error.to_string())
            .into_any_element()
    } else if processes.is_empty() {
        div()
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
        let mut rows = v_flex().gap_0p5().child(process_header(p)).children(
            sorted
                .iter()
                .take(PROCESS_LIMIT)
                .map(|process| process_row(process, p)),
        );
        if sorted.len() > PROCESS_LIMIT {
            rows = rows.child(div().text_xs().text_color(p.muted_foreground).child(
                rust_i18n::t!("container.stats.process_limit", count = PROCESS_LIMIT).to_string(),
            ));
        }
        rows.into_any_element()
    };
    stat_card(
        "container-stats-card-processes",
        IconName::ListTree,
        rust_i18n::t!("container.stats.processes").to_string(),
        summary,
        body,
        p,
    )
}

pub(crate) fn process_header(p: &Palette) -> AnyElement {
    let label = |text: String, width: f32, right: bool| {
        let mut element = div()
            .w(px(width))
            .flex_none()
            .text_xs()
            .text_color(p.muted_foreground)
            .child(text);
        if right {
            element = element.text_right();
        }
        element
    };
    h_flex()
        .gap_2()
        .items_center()
        .border_color(p.divider(p.border))
        .border_b_1()
        .pb_1()
        .child(label("PID".to_string(), 52.0, false))
        .child(label(
            rust_i18n::t!("container.stats.process_user").to_string(),
            72.0,
            false,
        ))
        .child(label("CPU".to_string(), 52.0, true))
        .child(label(
            rust_i18n::t!("container.stats.process_mem").to_string(),
            52.0,
            true,
        ))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("container.stats.process_command").to_string()),
        )
        .into_any_element()
}

pub(crate) fn process_row(process: &ContainerProcess, p: &Palette) -> AnyElement {
    h_flex()
        .gap_2()
        .items_center()
        .text_xs()
        .child(
            div()
                .w(px(52.0))
                .flex_none()
                .font_family(p.mono_font_family.clone())
                .text_color(p.muted_foreground)
                .child(process.pid.clone()),
        )
        .child(
            div()
                .w(px(72.0))
                .flex_none()
                .truncate()
                .text_color(p.foreground)
                .child(process.user.clone()),
        )
        .child(
            div()
                .w(px(52.0))
                .flex_none()
                .text_right()
                .text_color(usage_color(process.cpu_percent, p))
                .child(format!("{:.1}%", process.cpu_percent)),
        )
        .child(
            div()
                .w(px(52.0))
                .flex_none()
                .text_right()
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
