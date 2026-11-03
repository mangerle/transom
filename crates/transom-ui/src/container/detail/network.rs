//! 网络详情。

use super::*;
use transom_core::{ContainerCommand, ContainerRequestId};

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
    let connected_items: Vec<(String, String)> = connected
        .iter()
        .map(|item| (item.name.clone(), item.ipv4.clone().unwrap_or_default()))
        .collect();
    let mut rows = v_flex()
        .gap_1p5()
        .child(kv(p, rust_i18n::t!("container.field.id"), id.clone()))
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
    let disconnect_net_id = id.clone();
    let mut connected_block = v_flex()
        .gap_1()
        .child(
            div()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("container.field.connected").to_string()),
        );
    if connected_items.is_empty() {
        connected_block = connected_block.child(
            div()
                .text_sm()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("container.linked.none").to_string()),
        );
    } else {
        for (name, hint) in connected_items {
            let reveal_name = name.clone();
            let disconnect_name = name.clone();
            let net_id = disconnect_net_id.clone();
            let mut row = h_flex()
                .gap_2()
                .items_center()
                .child(
                    Icon::new(IconName::Container)
                        .xsmall()
                        .text_color(p.muted_foreground),
                )
                .child(
                    Button::new(ElementId::Name(
                        format!("network-container-link-{name}-{tab_id}").into(),
                    ))
                    .ghost()
                    .xsmall()
                    .label(name.clone())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.reveal_container(tab_id, reveal_name.clone(), cx);
                    })),
                );
            if !hint.is_empty() {
                row = row.child(div().text_xs().text_color(p.muted_foreground).child(hint));
            }
            row = row.child(div().flex_1());
            row = row.child(
                Button::new(ElementId::Name(
                    format!("network-disconnect-{name}-{tab_id}").into(),
                ))
                .ghost()
                .xsmall()
                .icon(IconName::Close)
                .tooltip(rust_i18n::t!("container.action.disconnect_network").to_string())
                .on_click(cx.listener(move |this, _, _, cx| {
                    let request_id = ContainerRequestId::new();
                    if let Some(workspace) = this.state.container_workspace_mut(tab_id) {
                        workspace.busy = true;
                    }
                    this.send_tracked(
                        tab_id,
                        request_id,
                        ContainerCommand::DisconnectNetwork {
                            tab_id,
                            request_id,
                            network: net_id.clone(),
                            container: disconnect_name.clone(),
                            force: false,
                        },
                    );
                    cx.notify();
                })),
            );
            connected_block = connected_block.child(row);
        }
    }
    rows = rows.child(connected_block);
    scroll_body(rows, p).into_any_element()
}
