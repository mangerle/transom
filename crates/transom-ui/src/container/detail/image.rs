//! 镜像详情。

use super::*;

pub(crate) fn render_image_detail(
    summary: Option<&transom_core::ContainerImageSummary>,
    details: Option<&ContainerImageDetails>,
    p: &Palette,
) -> AnyElement {
    if let Some(details) = details {
        let mut rows = v_flex()
            .gap_1p5()
            .child(kv(
                p,
                rust_i18n::t!("container.field.id"),
                details.id.clone(),
            ))
            .child(kv(
                p,
                rust_i18n::t!("container.field.tags"),
                details.tags.join("\n"),
            ))
            .child(kv(
                p,
                rust_i18n::t!("container.field.size"),
                crate::format::format_size(details.size),
            ))
            .child(kv(
                p,
                rust_i18n::t!("container.field.created"),
                format_unix(details.created),
            ))
            .child(kv(
                p,
                rust_i18n::t!("container.field.os_arch"),
                format!(
                    "{}/{}",
                    details.os.clone().unwrap_or_default(),
                    details.architecture.clone().unwrap_or_default()
                ),
            ))
            .child(kv(
                p,
                rust_i18n::t!("container.field.layers"),
                details.layers.to_string(),
            ));
        if !details.repo_digests.is_empty() {
            rows = rows.child(kv(
                p,
                rust_i18n::t!("container.field.repo_digests"),
                details.repo_digests.join("\n"),
            ));
        }
        if !details.parent.is_empty() {
            rows = rows.child(kv(
                p,
                rust_i18n::t!("container.field.parent"),
                details.parent.clone(),
            ));
        }
        if details.virtual_size > 0 {
            rows = rows.child(kv(
                p,
                rust_i18n::t!("container.field.virtual_size"),
                crate::format::format_size(details.virtual_size),
            ));
        }
        if !details.layer_digests.is_empty() {
            let mut layers = v_flex().gap_1().child(
                div().text_xs().text_color(p.muted_foreground).child(
                    rust_i18n::t!(
                        "container.detail.layer_list",
                        count = details.layer_digests.len()
                    )
                    .to_string(),
                ),
            );
            for (index, layer) in details.layer_digests.iter().enumerate() {
                layers = layers.child(
                    h_flex()
                        .gap_2()
                        .items_start()
                        .child(
                            div()
                                .w(px(22.0))
                                .flex_none()
                                .text_xs()
                                .text_color(p.muted_foreground)
                                .child(format!("{}", index + 1)),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_xs()
                                .font_family(p.mono_font_family.clone())
                                .child(layer.clone()),
                        ),
                );
            }
            rows = rows.child(layers);
        }
        return scroll_body(rows, p).into_any_element();
    }
    match summary {
        Some(summary) => {
            let rows = v_flex()
                .gap_1p5()
                .child(kv(
                    p,
                    rust_i18n::t!("container.field.id"),
                    summary.id.clone(),
                ))
                .child(kv(
                    p,
                    rust_i18n::t!("container.field.tags"),
                    summary.tags.join("\n"),
                ))
                .child(kv(
                    p,
                    rust_i18n::t!("container.field.size"),
                    crate::format::format_size(summary.size),
                ));
            scroll_body(rows, p).into_any_element()
        }
        None => centered_hint(
            rust_i18n::t!("container.detail.no_selection").to_string(),
            p,
        ),
    }
}
