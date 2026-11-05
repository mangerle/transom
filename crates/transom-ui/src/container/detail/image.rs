//! 镜像详情与构建历史页签呈现。
//!
//! 设计原理（Why）：
//! - 镜像详情分为 [基础信息 (Overview)] 与 [构建历史 (History)] 两个独立页签：
//!   1. 基础信息聚焦镜像静态元数据（ID、标签、摘要、大小、架构、创建时间与原始 Inspect JSON）；
//!   2. 构建历史全屏展开完整的分层时间线，呈现每一层的构建指令、增量大小、执行时间与对应标签，支持长指令换行与一键复制；
//! - 安全降级机制：
//!   当外部镜像剥离或缺失构建历史元数据时，自动降级为 RootFS.Layers 的 SHA256 散列指纹层展示，确保任何时候都能追踪每一层镜像分层。

use super::image_history::{copy_button, render_image_history};
use super::*;

/// 渲染镜像详情的主入口调度器。
pub(crate) fn render_image_detail(
    tab_id: TabId,
    summary: Option<&transom_core::ContainerImageSummary>,
    details: Option<&ContainerImageDetails>,
    overview_raw: bool,
    detail_tab: ContainerDetailTab,
    search_input: &Entity<InputState>,
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    match detail_tab {
        ContainerDetailTab::History => render_image_history(summary, details, p, cx),
        _ => render_image_overview(tab_id, summary, details, overview_raw, search_input, p, cx),
    }
}

/// 渲染镜像基础信息（Overview）页签。
fn render_image_overview(
    tab_id: TabId,
    summary: Option<&transom_core::ContainerImageSummary>,
    details: Option<&ContainerImageDetails>,
    raw: bool,
    search_input: &Entity<InputState>,
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    if let Some(details) = details {
        // 原始 Inspect JSON 视图
        if raw {
            return render_raw_inspect(tab_id, &details.inspect, search_input, p, cx);
        }

        let mut rows = v_flex().gap_2();

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

        let raw_btn = Button::new("image-overview-raw-inspect")
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
            IconName::Layers,
            p,
            Some(raw_btn),
            rows,
        );

        return scroll_body(v_flex().gap_3().child(card), p).into_any_element();
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
