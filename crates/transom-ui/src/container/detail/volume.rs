//! 数据卷详情。

use super::*;

pub(crate) fn render_volume_detail(
    tab_id: TabId,
    summary: Option<&transom_core::ContainerVolumeSummary>,
    details: Option<&ContainerVolumeDetails>,
    overview_raw: bool,
    search_input: &Entity<InputState>,
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    if overview_raw {
        let inspect_text = details.map(|d| d.inspect.as_str()).unwrap_or_default();
        return render_raw_inspect(tab_id, inspect_text, search_input, p, cx);
    }

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
    let browse_bar = h_flex().pb_1().child(
        Button::new("browse-volume-files")
            .primary()
            .small()
            .icon(IconName::FolderOpen)
            .label(rust_i18n::t!("container.files.browse").to_string())
            .on_click(cx.listener(move |this, _, _, cx| {
                if let Some(w) = this.state.container_workspace_mut(tab_id) {
                    w.detail_tab = ContainerDetailTab::Files;
                }
                this.load_container_dir(tab_id, "/".to_string());
                cx.notify();
            })),
    );
    let rows = v_flex()
        .gap_1p5()
        .child(browse_bar)
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
    let raw_btn = Button::new("volume-overview-raw-inspect")
        .xsmall()
        .ghost()
        .icon(IconName::Braces)
        .label(rust_i18n::t!("container.field.inspect").to_string())
        .on_click(cx.listener(move |this, _, _, cx| {
            if let Some(workspace) = this.state.container_workspace_mut(tab_id) {
                workspace.overview_raw = true;
            }
            cx.notify();
        }))
        .into_any_element();

    let card = super::overview_cards::overview_card(
        rust_i18n::t!("container.detail.category_identity").to_string(),
        IconName::HardDrive,
        p,
        Some(raw_btn),
        rows,
    );

    scroll_body(v_flex().gap_3().child(card), p).into_any_element()
}
