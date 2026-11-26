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

/// 解析后的网络身份字段，避免渲染函数同时处理摘要与详情两套可选来源。
struct NetworkIdentity {
    id: String,
    name: String,
    driver: String,
    scope: String,
    subnet: String,
    gateway: String,
    connected: Vec<(String, String)>,
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
    let Some(identity) = resolve_network_identity(summary, details) else {
        return centered_hint(
            rust_i18n::t!("container.detail.no_selection").to_string(),
            p,
        );
    };
    let rows = render_network_identity_rows(p, &identity, details);
    let rows = rows.child(render_connected_networks(
        view,
        &identity.id,
        &identity.connected,
        p,
    ));
    let raw_btn = render_network_raw_button(view);
    let card = super::overview_cards::overview_card(
        rust_i18n::t!("container.detail.category_identity").to_string(),
        AppIcon::Custom(CustomIcon::NETWORK),
        p,
        Some(raw_btn),
        rows,
    );
    scroll_body(v_flex().gap_3().child(card), p).into_any_element()
}

fn resolve_network_identity(
    summary: Option<&transom_core::ContainerNetworkSummary>,
    details: Option<&ContainerNetworkDetails>,
) -> Option<NetworkIdentity> {
    let source = if let Some(details) = details {
        (
            &details.summary,
            details.gateway.clone().unwrap_or_default(),
            details.connected.clone(),
        )
    } else {
        let summary = summary?;
        (summary, String::new(), Vec::new())
    };
    let (summary, gateway, connected) = source;
    Some(NetworkIdentity {
        id: summary.id.clone(),
        name: summary.name.clone(),
        driver: summary.driver.clone(),
        scope: summary.scope.clone(),
        subnet: summary.subnet.clone().unwrap_or_default(),
        gateway,
        connected: connected
            .iter()
            .map(|item| (item.name.clone(), item.ipv4.clone().unwrap_or_default()))
            .collect(),
    })
}

fn render_network_identity_rows(
    p: &Palette,
    identity: &NetworkIdentity,
    details: Option<&ContainerNetworkDetails>,
) -> Div {
    let mut rows = v_flex()
        .gap_1p5()
        .child(kv(
            p,
            rust_i18n::t!("container.field.id"),
            identity.id.clone(),
        ))
        .child(kv(
            p,
            rust_i18n::t!("container.field.name"),
            identity.name.clone(),
        ))
        .child(kv(
            p,
            rust_i18n::t!("container.field.driver"),
            identity.driver.clone(),
        ))
        .child(kv(
            p,
            rust_i18n::t!("container.field.scope"),
            identity.scope.clone(),
        ))
        .child(kv(
            p,
            rust_i18n::t!("container.field.subnet"),
            identity.subnet.clone(),
        ))
        .child(kv(
            p,
            rust_i18n::t!("container.field.gateway"),
            identity.gateway.clone(),
        ));
    if let Some(details) = details {
        rows = rows.child(render_network_detail_flags(p, details));
    }
    rows
}

fn render_network_detail_flags(p: &Palette, details: &ContainerNetworkDetails) -> Div {
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
    v_flex()
        .gap_1p5()
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
        .child(kv(p, rust_i18n::t!("container.field.labels"), labels))
}

fn render_connected_networks(
    view: &Entity<ContainerWorkspace>,
    network_id: &str,
    connected: &[(String, String)],
    p: &Palette,
) -> AnyElement {
    let mut block = v_flex().gap_1().child(
        div()
            .text_size(rems(typography::CAPTION))
            .text_color(p.muted_foreground)
            .child(rust_i18n::t!("container.field.connected").to_string()),
    );
    if connected.is_empty() {
        return block
            .child(
                div()
                    .text_size(rems(typography::BODY))
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("container.linked.none").to_string()),
            )
            .into_any_element();
    }
    for (name, hint) in connected {
        block = block.child(render_connected_network_row(
            view, network_id, name, hint, p,
        ));
    }
    block.into_any_element()
}

fn render_connected_network_row(
    view: &Entity<ContainerWorkspace>,
    network_id: &str,
    name: &str,
    hint: &str,
    p: &Palette,
) -> AnyElement {
    let reveal_view = view.clone();
    let disconnect_view = view.clone();
    let reveal_name = name.to_string();
    let disconnect_name = name.to_string();
    let net_id = network_id.to_string();
    let button_id = ElementId::Name(format!("network-container-link-{name}").into());
    let row = h_flex().gap_2().items_center().child(
        action_button(button_id, DomainIcon::CONTAINER, name.to_string())
            .ghost()
            .on_click(move |_, _, cx| {
                reveal_view.update(cx, |workspace, cx| {
                    workspace.reveal_container(reveal_name.clone(), cx);
                });
            }),
    );
    let row = if hint.is_empty() {
        row
    } else {
        row.child(
            div()
                .text_size(rems(typography::CAPTION))
                .text_color(p.muted_foreground)
                .child(hint.to_string()),
        )
    };
    row.child(div().flex_1())
        .child(
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
        )
        .into_any_element()
}

fn render_network_raw_button(view: &Entity<ContainerWorkspace>) -> AnyElement {
    let raw_view = view.clone();
    action_button(
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
    .into_any_element()
}
