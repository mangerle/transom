//! 镜像构建历史（History）页签全屏时间线呈现与安全降级。
//!
//! 设计原理（Why）：
//! - 镜像构建历史是排查镜像臃肿、确认构建步骤和安全审计的核心视图：
//!   1. 全屏展开完整的分层时间线，呈现每一层的构建指令、增量大小、执行时间与对应标签，支持长指令换行与一键复制；
//!   2. 自动清洗 Docker 内部生成的 `/bin/sh -c #(nop) ` 冗余指令前缀，将原生 RUN 指令还原为标准 Dockerfile 格式；
//! - 安全降级机制：
//!   当外部镜像剥离或缺失构建历史元数据时，自动降级为 RootFS.Layers 的 SHA256 散列指纹层展示，确保任何时候都能追踪每一层镜像分层。

use super::*;
use ::gpui::ClipboardItem;

/// 渲染镜像构建历史（History）独立页签时间线。
pub(crate) fn render_image_history(
    summary: Option<&transom_core::ContainerImageSummary>,
    details: Option<&ContainerImageDetails>,
    p: &Palette,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let Some(details) = details else {
        if summary.is_some() {
            return centered_hint(
                rust_i18n::t!("container.detail.loading_history").to_string(),
                p,
            );
        }
        return centered_hint(
            rust_i18n::t!("container.detail.no_selection").to_string(),
            p,
        );
    };

    // 优先展示详细的构建指令历史时间线
    if !details.history.is_empty() {
        let mut history_list = v_flex().gap_2();

        // 概览摘要栏
        history_list = history_list.child(
            h_flex()
                .justify_between()
                .items_center()
                .p_2()
                .rounded(p.radius.md)
                .bg(p.muted.opacity(0.4))
                .child(
                    div().text_xs().text_color(p.muted_foreground).child(
                        rust_i18n::t!(
                            "container.detail.history_total",
                            count = details.history.len()
                        )
                        .to_string(),
                    ),
                )
                .child(
                    div().text_xs().text_color(p.muted_foreground).child(
                        rust_i18n::t!(
                            "container.detail.total_size",
                            size = crate::format::format_size(details.size)
                        )
                        .to_string(),
                    ),
                ),
        );

        // 逐层时间线卡片
        for (index, item) in details.history.iter().enumerate() {
            let size_str = if item.size == 0 {
                "0 B".to_string()
            } else {
                format!("+ {}", crate::format::format_size(item.size))
            };
            let cmd_raw = if item.created_by.is_empty() {
                item.comment.clone()
            } else {
                item.created_by.clone()
            };
            let cmd_cleaned = clean_dockerfile_instruction(&cmd_raw);
            let tags_str = if !item.tags.is_empty() {
                format!("Tag: {}", item.tags.join(", "))
            } else {
                String::new()
            };

            let copy_cmd = copy_button(
                ElementId::Name(format!("copy-layer-cmd-{index}").into()),
                cmd_raw.clone(),
                rust_i18n::t!("container.detail.copy_instruction").to_string(),
                cx,
            );

            history_list = history_list.child(
                v_flex()
                    .gap_1p5()
                    .p_2()
                    .rounded(p.radius.md)
                    .bg(p.muted.opacity(0.35))
                    .border_1()
                    .border_color(p.border.opacity(0.5))
                    .child(
                        h_flex()
                            .justify_between()
                            .items_center()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .items_center()
                                    .child(
                                        div()
                                            .px_1p5()
                                            .py_0p5()
                                            .rounded(p.radius.sm)
                                            .bg(p.primary.opacity(0.15))
                                            .text_xs()
                                            .text_color(p.primary)
                                            .child(format!("#{}", index + 1)),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(if item.size > 0 {
                                                p.primary
                                            } else {
                                                p.muted_foreground
                                            })
                                            .child(size_str),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(p.muted_foreground)
                                            .child(format_unix(item.created)),
                                    )
                                    .children(if !tags_str.is_empty() {
                                        Some(
                                            div()
                                                .px_1p5()
                                                .py_0p5()
                                                .rounded(p.radius.sm)
                                                .bg(p.muted)
                                                .text_xs()
                                                .text_color(p.foreground)
                                                .child(tags_str),
                                        )
                                    } else {
                                        None
                                    }),
                            )
                            .child(copy_cmd),
                    )
                    .child(
                        div()
                            .p_1p5()
                            .rounded(p.radius.sm)
                            .bg(p.background)
                            .font_family(p.mono_font_family.clone())
                            .text_size(p.mono_font_size)
                            .text_color(p.foreground)
                            .child(cmd_cleaned),
                    ),
            );
        }

        return scroll_body(history_list, p).into_any_element();
    }

    // 安全降级：当镜像无详细构建历史时，展示 RootFS.Layers 的 SHA256 散列指纹层
    if !details.layer_digests.is_empty() {
        let mut layers_list = v_flex().gap_2();

        // 降级提示条
        layers_list = layers_list.child(
            v_flex()
                .gap_1()
                .p_2()
                .rounded(p.radius.md)
                .bg(p.primary.opacity(0.1))
                .border_1()
                .border_color(p.primary.opacity(0.3))
                .child(
                    div().text_xs().text_color(p.primary).child(
                        rust_i18n::t!("container.detail.history_fallback_hint").to_string(),
                    ),
                ),
        );

        for (index, digest) in details.layer_digests.iter().enumerate() {
            let copy_digest = copy_button(
                ElementId::Name(format!("copy-layer-digest-{index}").into()),
                digest.clone(),
                rust_i18n::t!("container.detail.copy_layer_digest").to_string(),
                cx,
            );

            layers_list = layers_list.child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .p_2()
                    .rounded(p.radius.md)
                    .bg(p.muted.opacity(0.35))
                    .border_1()
                    .border_color(p.border.opacity(0.5))
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .px_1p5()
                                    .py_0p5()
                                    .rounded(p.radius.sm)
                                    .bg(p.primary.opacity(0.15))
                                    .text_xs()
                                    .text_color(p.primary)
                                    .child(format!("#{}", index + 1)),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .font_family(p.mono_font_family.clone())
                                    .text_size(p.mono_font_size)
                                    .text_color(p.foreground)
                                    .child(digest.clone()),
                            ),
                    )
                    .child(copy_digest),
            );
        }

        return scroll_body(layers_list, p).into_any_element();
    }

    centered_hint(
        rust_i18n::t!("container.detail.no_history_info").to_string(),
        p,
    )
}

/// 清洗并美化 Docker 构建历史指令，去除内部包装命令前缀。
pub(crate) fn clean_dockerfile_instruction(raw: &str) -> String {
    let trimmed = raw.trim();
    if let Some(stripped) = trimmed.strip_prefix("/bin/sh -c #(nop) ") {
        stripped.trim().to_string()
    } else if let Some(stripped) = trimmed.strip_prefix("/bin/sh -c ") {
        format!("RUN {stripped}")
    } else if trimmed.is_empty() {
        "<empty layer>".to_string()
    } else {
        trimmed.to_string()
    }
}

/// 通用一键复制按钮辅助组件。
pub(crate) fn copy_button(
    id: ElementId,
    text: String,
    tooltip: String,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    Button::new(id)
        .xsmall()
        .ghost()
        .icon(IconName::Copy)
        .tooltip(tooltip)
        .on_click(cx.listener({
            let text = text.clone();
            move |this, _, window, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                this.show_toast(
                    rust_i18n::t!("container.row.copied", value = text).to_string(),
                    true,
                    window,
                    cx,
                );
            }
        }))
        .into_any_element()
}
