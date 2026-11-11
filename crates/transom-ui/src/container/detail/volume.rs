//! 数据卷详情。

use super::*;
use crate::container::{ContainerWorkspace, ContainerWorkspaceEvent};
use transom_ui_icons::AppIcon;
use transom_ui_icons::AppIconElement;
use transom_ui_icons::CustomIcon;
use transom_ui_icons::UiIcon;

/// 数据卷详情渲染参数。
pub(crate) struct ContainerVolumeView<'a> {
    pub(crate) summary: Option<&'a transom_core::ContainerVolumeSummary>,
    pub(crate) details: Option<&'a ContainerVolumeDetails>,
    pub(crate) overview_raw: bool,
    pub(crate) inspect: InspectSearch<'a>,
}

pub(crate) fn render_volume_detail(
    view: &Entity<ContainerWorkspace>,
    params: ContainerVolumeView<'_>,
    p: &Palette,
) -> AnyElement {
    let ContainerVolumeView {
        summary,
        details,
        overview_raw,
        inspect,
    } = params;
    if overview_raw {
        let inspect_text = details.map(|d| d.inspect.as_str()).unwrap_or_default();
        return render_raw_inspect(view, inspect_text, inspect, p);
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
    let browse_view = view.clone();
    let browse_bar = h_flex().pb_1().child(
        Button::new("browse-volume-files")
            .primary()
            .small()
            .child(AppIconElement::new(AppIcon::Ui(UiIcon::FOLDER)).small())
            .label(rust_i18n::t!("container.files.browse").to_string())
            .on_click(move |_, _, cx| {
                browse_view.update(cx, |_, cx| {
                    cx.emit(ContainerWorkspaceEvent::OpenVolumeFiles);
                });
            }),
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
            view,
            rust_i18n::t!("container.field.used_by").to_string(),
            used_by
                .into_iter()
                .map(|name| (name, String::new()))
                .collect(),
            p,
        ));
    let raw_view = view.clone();
    let raw_btn = Button::new("volume-overview-raw-inspect")
        .xsmall()
        .ghost()
        .child(AppIconElement::new(AppIcon::Ui(UiIcon::EDITOR_ONLY)).small())
        .label(rust_i18n::t!("container.field.inspect").to_string())
        .on_click(move |_, _, cx| {
            raw_view.update(cx, |workspace, cx| {
                workspace.state.overview_raw = true;
                cx.notify();
            });
        })
        .into_any_element();

    let card = super::overview_cards::overview_card(
        rust_i18n::t!("container.detail.category_identity").to_string(),
        AppIcon::Custom(CustomIcon::VOLUME),
        p,
        Some(raw_btn),
        rows,
    );

    scroll_body(v_flex().gap_3().child(card), p).into_any_element()
}
