//! 镜像详情与构建历史页签呈现。
//!
//! 设计原理（Why）：
//! - 镜像详情分为 [基础信息 (Overview)] 与 [构建历史 (History)] 两个独立页签：
//!   1. 基础信息聚焦镜像静态元数据（ID、标签、摘要、大小、架构、创建时间与原始 Inspect JSON）；
//!   2. 构建历史全屏展开完整的分层时间线，呈现每一层的构建指令、增量大小、执行时间与对应标签，支持长指令换行与一键复制；
//! - 安全降级机制：
//!   当外部镜像剥离或缺失构建历史元数据时，自动降级为 RootFS.Layers 的 SHA256 散列指纹层展示，确保任何时候都能追踪每一层镜像分层。

use super::*;
use super::image_history::{copy_button, render_image_history};

/// 渲染镜像详情的主入口调度器。
pub(crate) fn render_image_detail(
    summary: Option<&transom_core::ContainerImageSummary>,
    details: Option<&ContainerImageDetails>,
    overview_raw: bool,
    detail_tab: ContainerDetailTab,
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    match detail_tab {
        ContainerDetailTab::History => render_image_history(summary, details, p, cx),
        _ => render_image_overview(summary, details, overview_raw, p, cx),
    }
}

/// 渲染镜像基础信息（Overview）页签。
fn render_image_overview(
    summary: Option<&transom_core::ContainerImageSummary>,
    details: Option<&ContainerImageDetails>,
    raw: bool,
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    if let Some(details) = details {
        let mut rows = v_flex().gap_2();

        // 原始 Inspect JSON 视图
        if raw {
            let body: AnyElement = if details.inspect.is_empty() {
                div()
                    .text_sm()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("container.detail.no_inspect").to_string())
                    .into_any_element()
            } else {
                let inspect_text = details.inspect.clone();
                let copy_inspect = copy_button(
                    ElementId::Name("image-copy-inspect".into()),
                    inspect_text.clone(),
                    rust_i18n::t!("container.detail.copy_inspect").to_string(),
                    cx,
                );
                v_flex()
                    .gap_1p5()
                    .child(
                        h_flex()
                            .justify_between()
                            .items_center()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(p.muted_foreground)
                                    .child(
                                        rust_i18n::t!(
                                            "container.detail.inspect_bytes",
                                            bytes = inspect_text.len()
                                        )
                                        .to_string(),
                                    ),
                            )
                            .child(copy_inspect),
                    )
                    .child(
                        div()
                            .p_2()
                            .rounded(p.radius.md)
                            .bg(p.muted.opacity(0.3))
                            .font_family(p.mono_font_family.clone())
                            .text_size(p.mono_font_size)
                            .child(inspect_text),
                    )
                    .into_any_element()
            };
            rows = rows.child(body);
            return scroll_body(rows, p).into_any_element();
        }

        // 结构化基础元数据
        let id_copy = copy_button(
            ElementId::Name("image-copy-id".into()),
            details.id.clone(),
            rust_i18n::t!("container.row.copy_id").to_string(),
            cx,
        );
        rows = rows
            .child(
                h_flex()
                    .justify_between()
                    .items_start()
                    .child(div().flex_1().child(kv(
                        p,
                        rust_i18n::t!("container.field.id"),
                        details.id.clone(),
                    )))
                    .child(id_copy),
            )
            .child(kv(
                p,
                rust_i18n::t!("container.field.tags"),
                if details.tags.is_empty() {
                    "<none>".to_string()
                } else {
                    details.tags.join("\n")
                },
            ))
            .child(kv(
                p,
                rust_i18n::t!("container.field.size"),
                crate::format::format_size(details.size),
            ));

        if details.virtual_size > 0 && details.virtual_size != details.size {
            rows = rows.child(kv(
                p,
                rust_i18n::t!("container.field.virtual_size"),
                crate::format::format_size(details.virtual_size),
            ));
        }

        rows = rows
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
                    details.os.clone().unwrap_or_else(|| "linux".to_string()),
                    details
                        .architecture
                        .clone()
                        .unwrap_or_else(|| "amd64".to_string())
                ),
            ))
            .child(kv(
                p,
                rust_i18n::t!("container.field.layers"),
                rust_i18n::t!("container.detail.layer_count", count = details.layers).to_string(),
            ));

        if !details.repo_digests.is_empty() {
            let digests_str = details.repo_digests.join("\n");
            let digest_copy = copy_button(
                ElementId::Name("image-copy-digests".into()),
                digests_str.clone(),
                rust_i18n::t!("common.copy").to_string(),
                cx,
            );
            rows = rows.child(
                h_flex()
                    .justify_between()
                    .items_start()
                    .child(div().flex_1().child(kv(
                        p,
                        rust_i18n::t!("container.field.repo_digests"),
                        digests_str,
                    )))
                    .child(digest_copy),
            );
        }

        if !details.parent.is_empty() {
            let parent_copy = copy_button(
                ElementId::Name("image-copy-parent".into()),
                details.parent.clone(),
                rust_i18n::t!("common.copy").to_string(),
                cx,
            );
            rows = rows.child(
                h_flex()
                    .justify_between()
                    .items_start()
                    .child(div().flex_1().child(kv(
                        p,
                        rust_i18n::t!("container.field.parent"),
                        details.parent.clone(),
                    )))
                    .child(parent_copy),
            );
        }

        return scroll_body(rows, p).into_any_element();
    }

    match summary {
        Some(summary) => {
            let rows = v_flex()
                .gap_2()
                .child(kv(
                    p,
                    rust_i18n::t!("container.field.id"),
                    summary.id.clone(),
                ))
                .child(kv(
                    p,
                    rust_i18n::t!("container.field.tags"),
                    if summary.tags.is_empty() {
                        "<none>".to_string()
                    } else {
                        summary.tags.join("\n")
                    },
                ))
                .child(kv(
                    p,
                    rust_i18n::t!("container.field.size"),
                    crate::format::format_size(summary.size),
                ))
                .child(
                    div()
                        .pt_2()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(rust_i18n::t!("container.detail.loading_metadata").to_string()),
                );
            scroll_body(rows, p).into_any_element()
        }
        None => centered_hint(
            rust_i18n::t!("container.detail.no_selection").to_string(),
            p,
        ),
    }
}

