//! 端口映射可折叠表格卡片。

use super::*;
use crate::widgets::{status_badge, table_header};

pub(crate) fn ports_card(
    view: &Entity<ContainerWorkspace>,
    details: Option<&ContainerDetails>,
    browser_host: &str,
    collapsed: bool,
    chrome: &StatsChrome,
) -> AnyElement {
    let p = chrome.p;
    let Some(details) = details else {
        return stat_card(
            "container-stats-card-ports",
            IconName::Plug,
            rust_i18n::t!("container.stats.ports").to_string(),
            summary_text("—".to_string(), p.muted_foreground),
            div()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("container.stats.loading").to_string())
                .into_any_element(),
            p,
        );
    };

    let summary_str = rust_i18n::t!(
        "container.stats.ports_summary",
        published = details.ports.len(),
        exposed = details.exposed_ports.len()
    )
    .to_string();

    let header_btn = h_flex()
        .id("btn-toggle-stats-ports")
        .gap_2()
        .items_center()
        .justify_between()
        .w_full()
        .px_3()
        .py_2p5()
        .cursor_pointer()
        .on_click({
            let view = view.clone();
            move |_, _, cx| {
                view.update(cx, |workspace, cx| {
                    workspace.toggle_container_stats_ports();
                    cx.notify();
                });
            }
        })
        .child(
            h_flex()
                .gap_1p5()
                .items_center()
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
                    Icon::new(IconName::Plug)
                        .size(px(14.0))
                        .text_color(p.muted_foreground),
                )
                .child(
                    div()
                        .text_size(rems(0.6875))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(p.muted_foreground)
                        .child(rust_i18n::t!("container.stats.ports").to_string()),
                ),
        )
        .child(
            div()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(summary_str),
        );

    let mut card = v_flex()
        .id("container-stats-card-ports")
        .w_full()
        .rounded(px(10.0))
        .bg(p.surface.ghost)
        .overflow_hidden()
        .child(header_btn);

    if !collapsed {
        let is_empty = details.ports.is_empty() && details.exposed_ports.is_empty();
        let content: AnyElement = if is_empty {
            div()
                .px_3()
                .pb_3()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("container.stats.no_ports").to_string())
                .into_any_element()
        } else {
            let host = browser_host.to_string();
            let th = table_header(
                vec![
                    div()
                        .w(px(140.0))
                        .flex_none()
                        .child(rust_i18n::t!("container.stats.public_port").to_string())
                        .into_any_element(),
                    div()
                        .w(px(120.0))
                        .flex_none()
                        .child(rust_i18n::t!("container.stats.private_port").to_string())
                        .into_any_element(),
                    div()
                        .w(px(80.0))
                        .flex_none()
                        .child(rust_i18n::t!("container.stats.protocol").to_string())
                        .into_any_element(),
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(rust_i18n::t!("container.status").to_string())
                        .into_any_element(),
                ],
                p,
            );

            let mut table = v_flex().w_full().child(th);

            for port in &details.ports {
                let public_el: AnyElement = match port.public_port {
                    Some(public) => {
                        let url = format!("http://{host}:{public}");
                        let open_url = url.clone();
                        Button::new(format!("btn-stats-open-port-{public}"))
                            .ghost()
                            .xsmall()
                            .label(public.to_string())
                            .icon(IconName::ExternalLink)
                            .tooltip(url)
                            .on_click(move |_, _, _| {
                                if let Err(error) =
                                    transom_platform::opener::open_with_default_app(&open_url)
                                {
                                    log::warn!(
                                        "打开容器端口链接失败，地址: {open_url}，原因: {error}"
                                    );
                                }
                            })
                            .into_any_element()
                    }
                    None => div()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child("—")
                        .into_any_element(),
                };

                let row = h_flex()
                    .w_full()
                    .items_center()
                    .px_3()
                    .py_1p5()
                    .text_xs()
                    .child(div().w(px(140.0)).flex_none().child(public_el))
                    .child(
                        div()
                            .w(px(120.0))
                            .flex_none()
                            .font_family(p.mono_font_family.clone())
                            .text_color(p.foreground)
                            .child(port.private_port.to_string()),
                    )
                    .child(
                        div()
                            .w(px(80.0))
                            .flex_none()
                            .font_family(p.mono_font_family.clone())
                            .text_color(p.muted_foreground)
                            .child(port.protocol.to_lowercase()),
                    )
                    .child(div().flex_1().min_w_0().child(status_badge(
                        rust_i18n::t!("container.stats.published").to_string(),
                        p.success,
                        p,
                    )));
                table = table.child(row);
            }

            for exp in &details.exposed_ports {
                let parts: Vec<&str> = exp.split('/').collect();
                let private = parts.first().copied().unwrap_or(exp.as_str());
                let proto = parts.get(1).copied().unwrap_or("tcp");

                let row = h_flex()
                    .w_full()
                    .items_center()
                    .px_3()
                    .py_1p5()
                    .text_xs()
                    .child(
                        div()
                            .w(px(140.0))
                            .flex_none()
                            .text_color(p.muted_foreground)
                            .child("—"),
                    )
                    .child(
                        div()
                            .w(px(120.0))
                            .flex_none()
                            .font_family(p.mono_font_family.clone())
                            .text_color(p.foreground)
                            .child(private.to_string()),
                    )
                    .child(
                        div()
                            .w(px(80.0))
                            .flex_none()
                            .font_family(p.mono_font_family.clone())
                            .text_color(p.muted_foreground)
                            .child(proto.to_lowercase()),
                    )
                    .child(div().flex_1().min_w_0().child(status_badge(
                        rust_i18n::t!("container.stats.exposed").to_string(),
                        p.muted_foreground,
                        p,
                    )));
                table = table.child(row);
            }

            table.pb_2().into_any_element()
        };

        card = card.child(content);
    }

    card.into_any_element()
}
