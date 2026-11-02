//! 数据卷详情。

use super::*;

pub(crate) fn render_volume_detail(
    tab_id: TabId,
    summary: Option<&transom_core::ContainerVolumeSummary>,
    details: Option<&ContainerVolumeDetails>,
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let (name, driver, mountpoint, created, labels, options, used_by) =
        if let Some(details) = details {
            (
                details.summary.name.clone(),
                details.summary.driver.clone(),
                details.summary.mountpoint.clone(),
                details.summary.created.clone().unwrap_or_default(),
                details.summary.labels.clone(),
                details.summary.options.clone(),
                details.used_by.clone(),
            )
        } else if let Some(summary) = summary {
            (
                summary.name.clone(),
                summary.driver.clone(),
                summary.mountpoint.clone(),
                summary.created.clone().unwrap_or_default(),
                summary.labels.clone(),
                summary.options.clone(),
                Vec::new(),
            )
        } else {
            return centered_hint(
                rust_i18n::t!("container.detail.no_selection").to_string(),
                p,
            );
        };
    let labels = labels
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("\n");
    let options = options
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("\n");
    let rows = v_flex()
        .gap_1p5()
        .child(kv(p, rust_i18n::t!("container.field.name"), name))
        .child(kv(p, rust_i18n::t!("container.field.driver"), driver))
        .child(kv(p, rust_i18n::t!("container.field.options"), options))
        .child(kv(
            p,
            rust_i18n::t!("container.field.mountpoint"),
            mountpoint,
        ))
        .child(kv(p, rust_i18n::t!("container.field.created"), created))
        .child(kv(p, rust_i18n::t!("container.field.labels"), labels))
        .child(linked_containers(
            tab_id,
            rust_i18n::t!("container.field.used_by").to_string(),
            used_by
                .into_iter()
                .map(|name| (name, String::new()))
                .collect(),
            p,
            cx,
        ));
    scroll_body(rows, p).into_any_element()
}
