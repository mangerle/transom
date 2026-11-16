//! 端口映射可折叠表格卡片。

use super::*;
use crate::theme::tokens::typography;
use crate::widgets::{action_button, status_badge, table_header};
use transom_ui_icons::AppIcon;
use transom_ui_icons::AppIconElement;
use transom_ui_icons::CustomIcon;
use transom_ui_icons::UiIcon;

pub(crate) fn ports_card(
    view: &Entity<ContainerWorkspace>,
    details: Option<&ContainerDetails>,
    browser_host: &str,
    collapsed: bool,
    chrome: &StatsChrome,
) -> AnyElement {
    let p = chrome.p;
    let Some(details) = details else {
        return ports_loading_card(chrome);
    };
    let summary_str = rust_i18n::t!(
        "container.stats.ports_summary",
        published = details.ports.len(),
        exposed = details.exposed_ports.len()
    )
    .to_string();
    let header = ports_header(view, collapsed, &summary_str, p);
    let mut card = v_flex()
        .id("container-stats-card-ports")
        .w_full()
        .rounded(px(10.0))
        .bg(p.surface.ghost)
        .overflow_hidden()
        .child(header);
    if !collapsed {
        card = card.child(ports_body(details, browser_host, p));
    }
    card.into_any_element()
}

fn ports_loading_card(chrome: &StatsChrome) -> AnyElement {
    let p = chrome.p;
    stat_card(
        "container-stats-card-ports",
        AppIcon::Custom(CustomIcon::PORT),
        rust_i18n::t!("container.stats.ports").to_string(),
        summary_text("—".to_string(), p.muted_foreground),
        div()
            .text_xs()
            .text_color(p.muted_foreground)
            .child(rust_i18n::t!("container.stats.loading").to_string())
            .into_any_element(),
        p,
    )
}

fn ports_header(
    view: &Entity<ContainerWorkspace>,
    collapsed: bool,
    summary_str: &str,
    p: &Palette,
) -> AnyElement {
    let view = view.clone();
    h_flex()
        .id("btn-toggle-stats-ports")
        .gap_2()
        .items_center()
        .justify_between()
        .w_full()
        .px_3()
        .py_2p5()
        .cursor_pointer()
        .on_click(move |_, _, cx| {
            view.update(cx, |workspace, cx| {
                workspace.toggle_container_stats_ports();
                cx.notify();
            });
        })
        .child(
            h_flex()
                .gap_1p5()
                .items_center()
                .child(
                    AppIconElement::new(if collapsed {
                        AppIcon::Ui(UiIcon::CHEVRON_RIGHT)
                    } else {
                        AppIcon::Ui(UiIcon::CHEVRON_DOWN)
                    })
                    .with_rems(rems(crate::theme::tokens::control::COMPACT_ICON))
                    .text_color(p.muted_foreground),
                )
                .child(
                    AppIconElement::new(AppIcon::Custom(CustomIcon::PORT))
                        .with_rems(rems(crate::theme::tokens::control::COMPACT_ICON))
                        .text_color(p.muted_foreground),
                )
                .child(
                    div()
                        .text_size(rems(typography::CAPTION))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(p.muted_foreground)
                        .child(rust_i18n::t!("container.stats.ports").to_string()),
                ),
        )
        .child(
            div()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(summary_str.to_string()),
        )
        .into_any_element()
}

fn ports_body(details: &ContainerDetails, browser_host: &str, p: &Palette) -> AnyElement {
    if details.ports.is_empty() && details.exposed_ports.is_empty() {
        return div()
            .px_3()
            .pb_3()
            .text_xs()
            .text_color(p.muted_foreground)
            .child(rust_i18n::t!("container.stats.no_ports").to_string())
            .into_any_element();
    }
    let mut table = v_flex().w_full().child(ports_table_header(p));
    for port in &details.ports {
        table = table.child(published_port_row(port, browser_host, p));
    }
    for exp in &details.exposed_ports {
        table = table.child(exposed_port_row(exp, p));
    }
    table.pb_2().into_any_element()
}

fn ports_table_header(p: &Palette) -> AnyElement {
    table_header(
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
    )
    .into_any_element()
}

fn published_port_row(
    port: &transom_core::ContainerPort,
    browser_host: &str,
    p: &Palette,
) -> AnyElement {
    let public_el: AnyElement = match port.public_port {
        Some(public) => {
            let url = format!("http://{}:{}", browser_host, public);
            let open_url = url.clone();
            action_button(
                ElementId::Name(format!("btn-stats-open-port-{public}").into()),
                UiIcon::EXTERNAL_LINK,
                public.to_string(),
            )
            .ghost()
            .tooltip(url)
            .on_click(move |_, _, _| {
                if let Err(error) = transom_platform::opener::open_with_default_app(&open_url) {
                    log::warn!("打开容器端口链接失败，地址: {open_url}，原因: {error}");
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
    h_flex()
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
        )))
        .into_any_element()
}

fn exposed_port_row(exp: &str, p: &Palette) -> AnyElement {
    let parts: Vec<&str> = exp.split('/').collect();
    let private = parts.first().copied().unwrap_or(exp);
    let proto = parts.get(1).copied().unwrap_or("tcp");
    h_flex()
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
        )))
        .into_any_element()
}
