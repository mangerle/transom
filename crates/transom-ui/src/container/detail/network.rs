//! 网络详情。

use super::*;

pub(crate) fn render_network_detail(
    tab_id: TabId,
    summary: Option<&transom_core::ContainerNetworkSummary>,
    details: Option<&ContainerNetworkDetails>,
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let (id, name, driver, scope, subnet, gateway, connected) = if let Some(details) = details {
        (
            details.summary.id.clone(),
            details.summary.name.clone(),
            details.summary.driver.clone(),
            details.summary.scope.clone(),
            details.summary.subnet.clone().unwrap_or_default(),
            details.gateway.clone().unwrap_or_default(),
            details.connected.clone(),
        )
    } else if let Some(summary) = summary {
        (
            summary.id.clone(),
            summary.name.clone(),
            summary.driver.clone(),
            summary.scope.clone(),
            summary.subnet.clone().unwrap_or_default(),
            String::new(),
            Vec::new(),
        )
    } else {
        return centered_hint(
            rust_i18n::t!("container.detail.no_selection").to_string(),
            p,
        );
    };
    let connected_items = connected
        .iter()
        .map(|item| (item.name.clone(), item.ipv4.clone().unwrap_or_default()))
        .collect();
    let mut rows = v_flex()
        .gap_1p5()
        .child(kv(p, rust_i18n::t!("container.field.id"), id))
        .child(kv(p, rust_i18n::t!("container.field.name"), name))
        .child(kv(p, rust_i18n::t!("container.field.driver"), driver))
        .child(kv(p, rust_i18n::t!("container.field.scope"), scope))
        .child(kv(p, rust_i18n::t!("container.field.subnet"), subnet))
        .child(kv(p, rust_i18n::t!("container.field.gateway"), gateway));
    if let Some(details) = details {
        let flag = |value: bool| {
            rust_i18n::t!(if value {
                "container.bool.yes"
            } else {
                "container.bool.no"
            })
            .to_string()
        };
        let labels = details
            .labels
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>()
            .join("\n");
        rows = rows
            .child(kv(
                p,
                rust_i18n::t!("container.field.internal"),
                flag(details.internal),
            ))
            .child(kv(
                p,
                rust_i18n::t!("container.field.attachable"),
                flag(details.attachable),
            ))
            .child(kv(
                p,
                rust_i18n::t!("container.field.enable_ipv6"),
                flag(details.enable_ipv6),
            ))
            .child(kv(p, rust_i18n::t!("container.field.labels"), labels));
    }
    rows = rows.child(linked_containers(
        tab_id,
        rust_i18n::t!("container.field.connected").to_string(),
        connected_items,
        p,
        cx,
    ));
    scroll_body(rows, p).into_any_element()
}
