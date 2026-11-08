//! 镜像详情与构建历史页签呈现。
//!
//! 设计原理（Why）：
//! - 镜像详情分为 [基础信息 (Overview)] 与 [构建历史 (History)] 两个独立页签：
//!   1. 基础信息聚焦镜像静态元数据（ID、标签、摘要、大小、架构、创建时间与原始 Inspect JSON）；
//!   2. 构建历史全屏展开完整的分层时间线，呈现每一层的构建指令、增量大小、执行时间与对应标签，支持长指令换行与一键复制；
//! - 安全降级机制：
//!   当外部镜像剥离或缺失构建历史元数据时，自动降级为 RootFS.Layers 的 SHA256 散列指纹层展示，确保任何时候都能追踪每一层镜像分层。
//!
//! 渲染只持有工作区实体句柄与只读快照：复制提示写回实体 `notice`，原始视图
//! 切换直接改实体状态，不再依赖 `Context<AppShell>`。

use super::image_history::{copy_button, render_image_history};
use super::*;
use crate::container::ContainerWorkspace;

/// 镜像详情渲染参数。
pub(crate) struct ImageDetailView<'a> {
    pub(crate) summary: Option<&'a transom_core::ContainerImageSummary>,
    pub(crate) details: Option<&'a ContainerImageDetails>,
    pub(crate) overview_raw: bool,
    pub(crate) detail_tab: ContainerDetailTab,
    /// 原始 inspect 视图的搜索输入框与关键词快照。
    pub(crate) inspect: InspectSearch<'a>,
}

/// 渲染镜像详情的主入口调度器。
pub(crate) fn render_image_detail(
    view: &Entity<ContainerWorkspace>,
    params: ImageDetailView<'_>,
    p: &Palette,
) -> AnyElement {
    let ImageDetailView {
        summary,
        details,
        overview_raw,
        detail_tab,
        inspect,
    } = params;
    match detail_tab {
        ContainerDetailTab::History => render_image_history(view, summary, details, p),
        _ => render_image_overview(view, summary, details, overview_raw, inspect, p),
    }
}

/// 渲染镜像基础信息（Overview）页签。
fn render_image_overview(
    view: &Entity<ContainerWorkspace>,
    summary: Option<&transom_core::ContainerImageSummary>,
    details: Option<&ContainerImageDetails>,
    raw: bool,
    inspect: InspectSearch<'_>,
    p: &Palette,
) -> AnyElement {
    if let Some(details) = details {
        // 原始 Inspect JSON 视图
        if raw {
            return render_raw_inspect(view, &details.inspect, inspect, p);
        }

        let mut rows = v_flex().gap_2();

        // 结构化基础元数据
        let id_copy = copy_button(
            view,
            ElementId::Name("image-copy-id".into()),
            details.id.clone(),
            rust_i18n::t!("container.row.copy_id").to_string(),
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
                view,
                ElementId::Name("image-copy-digests".into()),
                digests_str.clone(),
                rust_i18n::t!("common.copy").to_string(),
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
                view,
                ElementId::Name("image-copy-parent".into()),
                details.parent.clone(),
                rust_i18n::t!("common.copy").to_string(),
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

        let raw_view = view.clone();
        let raw_btn = Button::new("image-overview-raw-inspect")
            .xsmall()
            .ghost()
            .icon(IconName::Braces)
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
