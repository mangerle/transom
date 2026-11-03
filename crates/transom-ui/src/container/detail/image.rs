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
        if !details.history.is_empty() {
            let mut history_box = v_flex().gap_1p5().child(
                div().text_xs().text_color(p.muted_foreground).child(
                    rust_i18n::t!(
                        "container.detail.history_list",
                        count = details.history.len()
                    )
                    .to_string(),
                ),
            );
            for (index, item) in details.history.iter().enumerate() {
                let size_str = if item.size == 0 {
                    "0 B".to_string()
                } else {
                    crate::format::format_size(item.size)
                };
                let cmd = if item.created_by.is_empty() {
                    item.comment.clone()
                } else {
                    item.created_by.clone()
                };
                let cmd_display = cmd.trim().strip_prefix("/bin/sh -c #(nop) ").unwrap_or(cmd.trim()).to_string();
                let tags_str = if !item.tags.is_empty() {
                    format!(" [{}]", item.tags.join(", "))
                } else {
                    String::new()
                };
                history_box = history_box.child(
                    v_flex()
                        .gap_0p5()
                        .p_1p5()
                        .rounded_md()
                        .bg(p.muted)
                        .child(
                            h_flex()
                                .gap_2()
                                .items_center()
                                .child(
                                    div()
                                        .w(px(24.0))
                                        .flex_none()
                                        .text_xs()
                                        .text_color(p.muted_foreground)
                                        .child(format!("#{}", index + 1)),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(if item.size > 0 { p.primary } else { p.muted_foreground })
                                        .child(size_str),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .text_xs()
                                        .text_color(p.muted_foreground)
                                        .child(format_unix(item.created)),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(p.muted_foreground)
                                        .child(tags_str),
                                ),
                        )
                        .child(
                            div()
                                .text_xs()
                                .font_family(p.mono_font_family.clone())
                                .text_color(p.foreground)
                                .child(cmd_display),
                        ),
                );
            }
            rows = rows.child(history_box);
        } else if !details.layer_digests.is_empty() {
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
