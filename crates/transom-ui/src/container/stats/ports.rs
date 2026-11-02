//! 端口映射卡片。

use super::*;

pub(crate) fn ports_card(details: Option<&ContainerDetails>, chrome: &StatsChrome) -> AnyElement {
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
    let mut rows = v_flex().gap_1();
    if details.ports.is_empty() && details.exposed_ports.is_empty() {
        rows = rows.child(
            div()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("container.stats.no_ports").to_string()),
        );
    }
    if !details.ports.is_empty() {
        rows = rows.child(
            v_flex()
                .gap_0p5()
                .child(
                    div()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(rust_i18n::t!("container.stats.published").to_string()),
                )
                .children(details.ports.iter().map(|port| {
                    let mapping = match port.public_port {
                        Some(public) => {
                            format!("{public} -> {}/{}", port.private_port, port.protocol)
                        }
                        None => rust_i18n::t!(
                            "container.stats.unpublished_port",
                            port = format!("{}/{}", port.private_port, port.protocol)
                        )
                        .to_string(),
                    };
                    div()
                        .text_xs()
                        .font_family(p.mono_font_family.clone())
                        .text_color(p.foreground)
                        .child(mapping)
                })),
        );
    }
    if !details.exposed_ports.is_empty() {
        rows = rows.child(
            v_flex()
                .gap_0p5()
                .child(
                    div()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(rust_i18n::t!("container.stats.exposed").to_string()),
                )
                .children(details.exposed_ports.iter().map(|port| {
                    div()
                        .text_xs()
                        .font_family(p.mono_font_family.clone())
                        .text_color(p.muted_foreground)
                        .child(port.clone())
                })),
        );
    }
    stat_card(
        "container-stats-card-ports",
        IconName::Plug,
        rust_i18n::t!("container.stats.ports").to_string(),
        summary_text(
            rust_i18n::t!(
                "container.stats.ports_summary",
                published = details.ports.len(),
                exposed = details.exposed_ports.len()
            )
            .to_string(),
            p.muted_foreground,
        ),
        rows.into_any_element(),
        p,
    )
}
