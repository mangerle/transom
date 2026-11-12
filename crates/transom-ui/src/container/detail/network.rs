//! 网络详情。

use super::*;
use crate::container::ContainerWorkspace;
use crate::theme::tokens::typography;
use crate::widgets::{IconButton, IconButtonSize, action_button};
use transom_ui_icons::AppIcon;
use transom_ui_icons::CustomIcon;
use transom_ui_icons::DomainIcon;
use transom_ui_icons::UiIcon;

/// 网络详情渲染参数。
pub(crate) struct ContainerNetworkView<'a> {
    pub(crate) summary: Option<&'a transom_core::ContainerNetworkSummary>,
    pub(crate) details: Option<&'a ContainerNetworkDetails>,
    pub(crate) overview_raw: bool,
    pub(crate) inspect: InspectSearch<'a>,
}

pub(crate) fn render_network_detail(
    view: &Entity<ContainerWorkspace>,
    params: ContainerNetworkView<'_>,
    p: &Palette,
) -> AnyElement {
    let ContainerNetworkView {
        summary,
        details,
        overview_raw,
        inspect,
    } = params;
    if overview_raw {
        let inspect_text = details.map(|d| d.inspect.as_str()).unwrap_or_default();
        return render_raw_inspect(view, inspect_text, inspect, p);
    }

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
    let mut connected_block = v_flex().gap_1().child(
        div()
            .text_size(rems(typography::CAPTION))
            .text_color(p.muted_foreground)
            .child(rust_i18n::t!("container.field.connected").to_string()),
    );
    if connected_items.is_empty() {
        connected_block = connected_block.child(
            div()
                .text_size(rems(typography::BODY))
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("container.linked.none").to_string()),
        );
    } else {
        for (name, hint) in connected_items {
            let reveal_name = name.clone();
            let disconnect_name = name.clone();
            let net_id = disconnect_net_id.clone();
            let reveal_view = view.clone();
            let disconnect_view = view.clone();
            let mut row = h_flex().gap_2().items_center().child(
                action_button(
                    ElementId::Name(format!("network-container-link-{name}").into()),
                    DomainIcon::CONTAINER,
                    name.clone(),
                )
                .ghost()
                .on_click(move |_, _, cx| {
                    reveal_view.update(cx, |workspace, cx| {
                        workspace.reveal_container(reveal_name.clone(), cx);
                    });
                }),
            );
            if !hint.is_empty() {
                row = row.child(
                    div()
                        .text_size(rems(typography::CAPTION))
                        .text_color(p.muted_foreground)
                        .child(hint),
                );
            }
            row = row.child(div().flex_1());
            row = row.child(
                IconButton::new(
                    ElementId::Name(format!("network-disconnect-{name}").into()),
                    UiIcon::CLOSE,
                    rust_i18n::t!("container.action.disconnect_network").to_string(),
                )
                .size(IconButtonSize::RowAction)
                .render()
                .on_click(move |_, _, cx| {
                    disconnect_view.update(cx, |workspace, cx| {
                        workspace.disconnect_network(net_id.clone(), disconnect_name.clone());
                        cx.notify();
                    });
                }),
            );
            connected_block = connected_block.child(row);
        }
    }
    rows = rows.child(connected_block);
    let raw_view = view.clone();
    let raw_btn = action_button(
        "network-overview-raw-inspect",
        UiIcon::EDITOR_ONLY,
        rust_i18n::t!("container.field.inspect").to_string(),
    )
    .ghost()
    .on_click(move |_, _, cx| {
        raw_view.update(cx, |workspace, cx| {
            workspace.state.overview_raw = true;
            cx.notify();
        });
    })
    .into_any_element();

    let card = super::overview_cards::overview_card(
        rust_i18n::t!("container.detail.category_identity").to_string(),
        AppIcon::Custom(CustomIcon::NETWORK),
        p,
        Some(raw_btn),
        rows,
    );

    scroll_body(v_flex().gap_3().child(card), p).into_any_element()
}
