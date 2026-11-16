//! 镜像构建历史（History）页签全屏时间线呈现与安全降级。
//!
//! 设计原理（Why）：
//! - 镜像构建历史是排查镜像臃肿、确认构建步骤和安全审计的核心视图：
//!   1. 全屏展开完整的分层时间线，呈现每一层的构建指令、增量大小、执行时间与对应标签，支持长指令换行与一键复制；
//!   2. 自动清洗 Docker 内部生成的 `/bin/sh -c #(nop) ` 冗余指令前缀，将原生 RUN 指令还原为标准 Dockerfile 格式；
//! - 安全降级机制：
//!   当外部镜像剥离或缺失构建历史元数据时，自动降级为 RootFS.Layers 的 SHA256 散列指纹层展示，确保任何时候都能追踪每一层镜像分层。

use super::*;
use crate::container::ContainerWorkspace;
use gpui_kit::ClipboardItem;
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

fn render_history_layer_card(
    view: &Entity<ContainerWorkspace>,
    index: usize,
    item: &transom_core::ContainerImageLayer,
    layer_advices: &[ImageSlimmingAdvice],
    p: &Palette,
) -> AnyElement {
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
        view,
        ElementId::Name(format!("copy-layer-cmd-{index}").into()),
        cmd_raw,
        rust_i18n::t!("container.detail.copy_instruction").to_string(),
    );

    let mut card = v_flex()
        .gap_1p5()
        .p_2()
        .rounded(p.radius.md)
        .bg(p.surface.active)
        .border_1()
        .border_color(p.stroke.strong)
        .child(render_layer_card_header(
            index,
            &size_str,
            item.size > 0,
            item.created,
            &tags_str,
            copy_cmd,
            p,
        ))
        .child(
            div()
                .p_1p5()
                .rounded(p.radius.sm)
                .bg(p.background)
                .font_family(p.mono_font_family.clone())
                .text_size(p.mono_font_size)
                .text_color(p.foreground)
                .child(cmd_cleaned),
        );

    for adv in layer_advices {
        card = card.child(render_layer_advice_badge(adv, p));
    }
    card.into_any_element()
}

fn render_layer_card_header(
    index: usize,
    size_str: &str,
    has_size: bool,
    created: i64,
    tags_str: &str,
    copy_cmd: AnyElement,
    p: &Palette,
) -> AnyElement {
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
                        .text_color(if has_size {
                            p.primary
                        } else {
                            p.muted_foreground
                        })
                        .child(size_str.to_string()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(format_unix(created)),
                )
                .children((!tags_str.is_empty()).then(|| {
                    div()
                        .px_1p5()
                        .py_0p5()
                        .rounded(p.radius.sm)
                        .bg(p.muted)
                        .text_xs()
                        .text_color(p.foreground)
                        .child(tags_str.to_string())
                })),
        )
        .child(copy_cmd)
        .into_any_element()
}

fn render_layer_advice_badge(adv: &ImageSlimmingAdvice, p: &Palette) -> AnyElement {
    let badge_color = if adv.is_warning { p.danger } else { p.warning };
    h_flex()
        .gap_2()
        .p_1p5()
        .rounded(p.radius.sm)
        .bg(badge_color.opacity(0.12))
        .items_start()
        .child(
            AppIconElement::new(if adv.is_warning {
                AppIcon::Ui(UiIcon::WARNING)
            } else {
                AppIcon::Custom(CustomIcon::INFO)
            })
            .with_size(px(14.0))
            .text_color(badge_color),
        )
        .child(
            v_flex()
                .gap_0p5()
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(badge_color)
                        .child(adv.title.clone()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(adv.suggestion.clone()),
                ),
        )
        .into_any_element()
}

fn render_layer_digests_fallback(
    view: &Entity<ContainerWorkspace>,
    layer_digests: &[String],
    p: &Palette,
) -> AnyElement {
    let mut layers_list = v_flex().gap_2().child(
        v_flex()
            .gap_1()
            .p_2()
            .rounded(p.radius.md)
            .bg(p.primary.opacity(0.1))
            .border_1()
            .border_color(p.primary.opacity(0.3))
            .child(
                div()
                    .text_xs()
                    .text_color(p.primary)
                    .child(rust_i18n::t!("container.detail.history_fallback_hint").to_string()),
            ),
    );

    for (index, digest) in layer_digests.iter().enumerate() {
        let copy_digest = copy_button(
            view,
            ElementId::Name(format!("copy-layer-digest-{index}").into()),
            digest.clone(),
            rust_i18n::t!("container.detail.copy_layer_digest").to_string(),
        );
        layers_list = layers_list.child(
            h_flex()
                .justify_between()
                .items_center()
                .p_2()
                .rounded(p.radius.md)
                .bg(p.surface.active)
                .border_1()
                .border_color(p.stroke.strong)
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
    scroll_body(layers_list, p).into_any_element()
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
///
/// 设计原理（Why）：复制只改剪贴板与工作区 `notice`，提示由外壳渲染帧统一转成
/// 通知；按钮因此只持有工作区实体句柄，不再需要 `Context<AppShell>`。
pub(crate) fn copy_button(
    view: &Entity<ContainerWorkspace>,
    id: ElementId,
    text: String,
    tooltip: String,
) -> AnyElement {
    let view = view.clone();
    crate::widgets::IconButton::new(id, UiIcon::COPY, tooltip)
        .size(crate::widgets::IconButtonSize::RowAction)
        .render()
        .on_click(move |_, _, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
            let message = rust_i18n::t!("container.row.copied", value = text).to_string();
            view.update(cx, |workspace, cx| {
                workspace.state.notice = Some(message);
                cx.notify();
            });
        })
        .into_any_element()
}

/// 镜像层瘦身与最佳实践建议诊断结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ImageSlimmingAdvice {
    pub(crate) is_warning: bool,
    pub(crate) title: String,
    pub(crate) suggestion: String,
}

/// 针对单层镜像指令与增量体积进行静态启发式分析，产出针对性的瘦身与最佳实践建议。
///
/// ### 设计初衷 (Why)
/// 容器开发者在编写 Dockerfile 时常因疏忽遗留包管理器下载缓存或未剥离构建产物，
/// 导致镜像体积膨胀数十至数百兆。通过在前端直接分析构建历史，即时给出可落地的瘦身优化方案。
///
/// ### 核心优势
/// 纯本地静态匹配启发式规则（包管理器缓存、推荐依赖、大层预警），零网络开销即可辅助开发者快速定位臃肿根源。
///
/// ### 代价与局限
/// 依赖构建历史中的原始指令文本；若镜像被打平（squash）或元数据被外部工具剥离，部分规则将自动静默退化。
pub(crate) fn inspect_layer_slimming(
    cmd_raw: &str,
    layer_size: u64,
    total_image_size: u64,
) -> Vec<ImageSlimmingAdvice> {
    let mut advice = Vec::new();
    let lower = cmd_raw.to_ascii_lowercase();

    // 1. 单层大文件体积检查 (超过 100MB 或超过总镜像大小的 40%)
    if layer_size >= 100 * 1024 * 1024
        || (total_image_size > 0
            && layer_size * 100 / total_image_size >= 40
            && layer_size > 30 * 1024 * 1024)
    {
        advice.push(ImageSlimmingAdvice {
            is_warning: true,
            title: "大体积镜像层".to_string(),
            suggestion: format!(
                "本层增量达 {}，建议检查是否打包了非必要的大型二进制、测试资源或未清理的构建依赖，可考虑多阶段构建或只复制产物。",
                crate::format::format_size(layer_size)
            ),
        });
    }

    // 2. APT 缓存未清理
    if lower.contains("apt-get install") || lower.contains("apt install") {
        if !lower.contains("rm -rf /var/lib/apt/lists") && !lower.contains("rm -rf /var/cache/apt")
        {
            advice.push(ImageSlimmingAdvice {
                is_warning: true,
                title: "APT 索引缓存残留".to_string(),
                suggestion: "检测到 apt 安装指令但未清理索引，建议在同一 RUN 指令末尾追加 `&& rm -rf /var/lib/apt/lists/*` 降低镜像体积。".to_string(),
            });
        }
        if !lower.contains("--no-install-recommends") {
            advice.push(ImageSlimmingAdvice {
                is_warning: false,
                title: "包含非必要推荐包".to_string(),
                suggestion: "建议为 apt-get install 添加 `--no-install-recommends` 参数，避免安装文档与额外推荐包。".to_string(),
            });
        }
    }

    // 3. Alpine APK 缓存未跳过
    if lower.contains("apk add") && !lower.contains("--no-cache") {
        advice.push(ImageSlimmingAdvice {
            is_warning: true,
            title: "Alpine APK 缓存未跳过".to_string(),
            suggestion:
                "建议使用 `apk add --no-cache` 代替常规安装，避免生成 `/var/cache/apk/*` 临时索引。"
                    .to_string(),
        });
    }

    // 4. Python Pip 缓存
    if lower.contains("pip install") && !lower.contains("--no-cache-dir") {
        advice.push(ImageSlimmingAdvice {
            is_warning: true,
            title: "Python Pip 缓存残留".to_string(),
            suggestion:
                "建议为 pip install 添加 `--no-cache-dir` 参数，避免将 wheel 和下载缓存写入只读层。"
                    .to_string(),
        });
    }

    // 5. NPM/Yarn 缓存
    if (lower.contains("npm install") || lower.contains("npm i "))
        && !lower.contains("cache clean")
        && layer_size > 20 * 1024 * 1024
    {
        advice.push(ImageSlimmingAdvice {
            is_warning: false,
            title: "Node 模块缓存建议清理".to_string(),
            suggestion:
                "建议在依赖安装后执行 `npm cache clean --force` 并使用 `--omit=dev` 剔除开发依赖。"
                    .to_string(),
        });
    }

    advice
}

#[cfg(test)]
mod tests {
    use super::inspect_layer_slimming;

    #[test]
    fn test_inspect_layer_slimming_apt_cache() {
        let cmd = "RUN apt-get update && apt-get install -y curl git";
        let advices = inspect_layer_slimming(cmd, 30 * 1024 * 1024, 100 * 1024 * 1024);
        assert!(advices.iter().any(|a| a.title == "APT 索引缓存残留"));
        assert!(advices.iter().any(|a| a.title == "包含非必要推荐包"));
    }

    #[test]
    fn test_inspect_layer_slimming_large_layer() {
        let cmd = "COPY big_model.bin /app/";
        let advices = inspect_layer_slimming(cmd, 150 * 1024 * 1024, 200 * 1024 * 1024);
        assert!(advices.iter().any(|a| a.title == "大体积镜像层"));
    }

    #[test]
    fn test_inspect_layer_slimming_clean_pip() {
        let cmd = "RUN pip install --no-cache-dir -r requirements.txt";
        let advices = inspect_layer_slimming(cmd, 10 * 1024 * 1024, 200 * 1024 * 1024);
        assert!(advices.is_empty());
    }
}
