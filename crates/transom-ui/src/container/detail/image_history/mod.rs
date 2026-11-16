//! 镜像构建历史（History）页签全屏时间线呈现与安全降级。
//!
//! 设计原理（Why）：
//! - 镜像构建历史是排查镜像臃肿、确认构建步骤和安全审计的核心视图：
//!   1. 全屏展开完整的分层时间线，呈现每一层的构建指令、增量大小、执行时间与对应标签，支持长指令换行与一键复制；
//!   2. 自动清洗 Docker 内部生成的 `/bin/sh -c #(nop) ` 冗余指令前缀，将原生 RUN 指令还原为标准 Dockerfile 格式；
//! - 安全降级机制：
//!   当外部镜像剥离或缺失构建历史元数据时，自动降级为 RootFS.Layers 的 SHA256 散列指纹层展示，确保任何时候都能追踪每一层镜像分层。

use super::*;

mod fallback;
mod layer_card;
mod slimming;

use fallback::render_layer_digests_fallback;
pub(crate) use fallback::{clean_dockerfile_instruction, copy_button};
use layer_card::render_history_layer_card;
pub(crate) use slimming::{ImageSlimmingAdvice, inspect_layer_slimming};

use crate::container::ContainerWorkspace;
use transom_ui_icons::AppIcon;
use transom_ui_icons::AppIconElement;
use transom_ui_icons::CustomIcon;
use transom_ui_icons::UiIcon;

/// 渲染镜像构建历史（History）独立页签时间线。
pub(crate) fn render_image_history(
    view: &Entity<ContainerWorkspace>,
    summary: Option<&transom_core::ContainerImageSummary>,
    details: Option<&ContainerImageDetails>,
    p: &Palette,
) -> AnyElement {
    let Some(details) = details else {
        let key = if summary.is_some() {
            "container.detail.loading_history"
        } else {
            "container.detail.no_selection"
        };
        return centered_hint(rust_i18n::t!(key).to_string(), p);
    };

    if !details.history.is_empty() {
        return render_history_timeline(view, details, p);
    }
    if !details.layer_digests.is_empty() {
        return render_layer_digests_fallback(view, &details.layer_digests, p);
    }
    centered_hint(
        rust_i18n::t!("container.detail.no_history_info").to_string(),
        p,
    )
}

fn render_history_timeline(
    view: &Entity<ContainerWorkspace>,
    details: &ContainerImageDetails,
    p: &Palette,
) -> AnyElement {
    let all_advices: Vec<Vec<ImageSlimmingAdvice>> = details
        .history
        .iter()
        .map(|item| {
            let cmd_raw = if item.created_by.is_empty() {
                &item.comment
            } else {
                &item.created_by
            };
            inspect_layer_slimming(cmd_raw, item.size, details.size)
        })
        .collect();
    let total_advice_count: usize = all_advices.iter().map(|a| a.len()).sum();

    let mut history_list = v_flex().gap_2().child(render_history_summary_bar(
        details.history.len(),
        total_advice_count,
        details.size,
        p,
    ));
    for (index, item) in details.history.iter().enumerate() {
        history_list = history_list.child(render_history_layer_card(
            view,
            index,
            item,
            &all_advices[index],
            p,
        ));
    }
    scroll_body(history_list, p).into_any_element()
}

fn render_history_summary_bar(
    layer_count: usize,
    total_advice_count: usize,
    total_size: u64,
    p: &Palette,
) -> AnyElement {
    h_flex()
        .justify_between()
        .items_center()
        .p_2()
        .rounded(p.radius.md)
        .bg(p.surface.pressed)
        .child(div().text_xs().text_color(p.muted_foreground).child(
            rust_i18n::t!("container.detail.history_total", count = layer_count).to_string(),
        ))
        .children((total_advice_count > 0).then(|| {
            div()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(p.warning)
                .child(format!("发现 {total_advice_count} 项瘦身建议"))
        }))
        .child(
            div().text_xs().text_color(p.muted_foreground).child(
                rust_i18n::t!(
                    "container.detail.total_size",
                    size = crate::format::format_size(total_size)
                )
                .to_string(),
            ),
        )
        .into_any_element()
}
