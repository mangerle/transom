//! 远端镜像市场探索弹窗专属纯视图组件。
//!
//! 设计原理（Why）：
//! - 将镜像搜索与市场浏览的纯展示逻辑（标题栏、热门搜索胶囊、特性指南卡片、搜索结果卡片等）
//!   从 `hub_dialog.rs` 中剥离解耦，确保单一职责与单文件行数精简；
//! - 遵循纯组件契约，只接收纯数据与行为闭包，杜绝在渲染过程中反查或重入借用宿主实体；
//! - 采用统一设计令牌与微交互规范，提供现代化、信息层级分明的市场探索体验。

use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::ImageSearchResult;

/// 常用热门探索镜像标签列表。
pub(super) const POPULAR_IMAGE_TAGS: &[&str] = &[
    "nginx", "redis", "mysql", "postgres", "node", "alpine", "python", "ubuntu", "golang",
];

/// 渲染镜像市场弹窗顶部 Header。
pub(super) fn render_hub_header(current_registry_name: &str, p: &Palette) -> AnyElement {
    h_flex()
        .w_full()
        .items_center()
        .justify_between()
        .pb_3()
        .border_b_1()
        .border_color(p.border.opacity(0.12))
        .child(
            h_flex()
                .gap_3()
                .items_center()
                .child(
                    v_flex()
                        .w(px(34.0))
                        .h(px(34.0))
                        .rounded(p.radius.md)
                        .bg(p.primary.opacity(0.12))
                        .items_center()
                        .justify_center()
                        .child(
                            Icon::new(IconName::Globe)
                                .with_size(px(18.0))
                                .text_color(p.primary),
                        ),
                )
                .child(
                    v_flex()
                        .gap_0p5()
                        .child(
                            div()
                                .text_base()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(p.foreground)
                                .child(rust_i18n::t!("container.hub.title").to_string()),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(p.muted_foreground)
                                .child(rust_i18n::t!("container.hub.subtitle").to_string()),
                        ),
                ),
        )
        .child(
            h_flex()
                .gap_1p5()
                .items_center()
                .px_2p5()
                .py_1()
                .rounded(p.radius.full)
                .bg(p.muted.opacity(0.08))
                .border_1()
                .border_color(p.border.opacity(0.15))
                .child(
                    div()
                        .w(px(6.0))
                        .h(px(6.0))
                        .rounded(p.radius.full)
                        .bg(p.success),
                )
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(p.muted_foreground)
                        .child(
                            rust_i18n::t!(
                                "container.hub.current_registry",
                                name = current_registry_name
                            )
                            .to_string(),
                        ),
                ),
        )
        .into_any_element()
}

/// 渲染热门推荐镜像胶囊行。
pub(super) fn render_popular_chips<F>(on_select: F, p: &Palette) -> AnyElement
where
    F: Fn(&str, &mut Window, &mut App) + 'static + Clone,
{
    let mut row = h_flex()
        .w_full()
        .gap_1p5()
        .items_center()
        .flex_wrap()
        .pt_1()
        .child(
            h_flex()
                .gap_1()
                .items_center()
                .child(
                    Icon::new(IconName::Sparkles)
                        .with_size(px(12.0))
                        .text_color(p.muted_foreground),
                )
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(p.muted_foreground)
                        .child(rust_i18n::t!("container.hub.popular_label").to_string()),
                ),
        );

    for &tag in POPULAR_IMAGE_TAGS {
        let tag_name = tag.to_string();
        let on_click_fn = on_select.clone();
        let chip = Button::new(ElementId::Name(format!("hub-chip-{}", tag).into()))
            .ghost()
            .xsmall()
            .label(tag_name.clone())
            .on_click(move |_, window, cx| {
                on_click_fn(&tag_name, window, cx);
            });

        row = row.child(chip);
    }

    row.into_any_element()
}

/// 格式化星标数字（如 12500 -> "12.5k"）。
pub(super) fn format_star_count(stars: u64) -> String {
    if stars >= 1_000_000 {
        format!("{:.1}M", stars as f64 / 1_000_000.0)
    } else if stars >= 1_000 {
        format!("{:.1}k", stars as f64 / 1_000.0)
    } else {
        stars.to_string()
    }
}

/// 渲染初始未搜索时的探索指南面板。
pub(super) fn render_hub_welcome_guide(p: &Palette) -> AnyElement {
    let guide_card = |icon: IconName, icon_color: Hsla, title: String, desc: String| {
        v_flex()
            .flex_1()
            .min_w_0()
            .p_3()
            .gap_2()
            .rounded(p.radius.lg)
            .border_1()
            .border_color(p.border.opacity(0.15))
            .bg(p.muted.opacity(0.03))
            .hover(|s| s.bg(p.muted.opacity(0.06)))
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        v_flex()
                            .w(px(26.0))
                            .h(px(26.0))
                            .rounded(p.radius.md)
                            .bg(icon_color.opacity(0.12))
                            .items_center()
                            .justify_center()
                            .child(Icon::new(icon).with_size(px(14.0)).text_color(icon_color)),
                    )
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(p.foreground)
                            .child(title),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .line_clamp(3)
                    .child(desc),
            )
    };

    v_flex()
        .w_full()
        .h(px(320.0))
        .justify_center()
        .items_center()
        .gap_4()
        .child(
            v_flex()
                .items_center()
                .gap_1p5()
                .child(
                    v_flex()
                        .w(px(48.0))
                        .h(px(48.0))
                        .rounded(p.radius.full)
                        .bg(p.primary.opacity(0.08))
                        .items_center()
                        .justify_center()
                        .child(
                            Icon::new(IconName::Globe)
                                .with_size(px(24.0))
                                .text_color(p.primary),
                        ),
                )
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(p.foreground)
                        .child(rust_i18n::t!("container.hub.welcome_title").to_string()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(rust_i18n::t!("container.hub.welcome_subtitle").to_string()),
                ),
        )
        .child(
            h_flex()
                .w_full()
                .gap_3()
                .child(guide_card(
                    IconName::ShieldCheck,
                    p.success,
                    rust_i18n::t!("container.hub.guide_official_title").to_string(),
                    rust_i18n::t!("container.hub.guide_official_desc").to_string(),
                ))
                .child(guide_card(
                    IconName::Star,
                    p.warning,
                    rust_i18n::t!("container.hub.guide_stars_title").to_string(),
                    rust_i18n::t!("container.hub.guide_stars_desc").to_string(),
                ))
                .child(guide_card(
                    IconName::Download,
                    p.primary,
                    rust_i18n::t!("container.hub.guide_pull_title").to_string(),
                    rust_i18n::t!("container.hub.guide_pull_desc").to_string(),
                )),
        )
        .into_any_element()
}

/// 渲染搜索结果列表顶部状态统计条。
pub(super) fn render_hub_results_header(
    count: usize,
    registry_name: &str,
    p: &Palette,
) -> AnyElement {
    h_flex()
        .w_full()
        .justify_between()
        .items_center()
        .px_1()
        .py_1p5()
        .child(
            h_flex()
                .gap_1p5()
                .items_center()
                .child(
                    Icon::new(IconName::Layers)
                        .with_size(px(13.0))
                        .text_color(p.muted_foreground),
                )
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(p.foreground)
                        .child(
                            rust_i18n::t!("container.hub.results_count", count = count).to_string(),
                        ),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(format!("({})", registry_name)),
                ),
        )
        .child(
            div()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("container.hub.empty_hint").to_string()),
        )
        .into_any_element()
}

/// 渲染单个远端镜像搜索结果卡片。
pub(super) fn render_hub_result_card<F>(
    item: &ImageSearchResult,
    on_pull: F,
    p: &Palette,
) -> AnyElement
where
    F: Fn(&mut Window, &mut App) + 'static,
{
    let name = item.name.clone();
    let desc = if item.description.is_empty() {
        rust_i18n::t!("container.hub.no_description").to_string()
    } else {
        item.description.clone()
    };
    let stars_str = format_star_count(item.star_count);
    let is_official = item.is_official;
    let is_automated = item.is_automated;
    let pull_ref = item.name.clone();

    // 智能拆分命名空间与镜像名称（如 bitnami/redis）
    let (namespace_part, repo_part) = if let Some((ns, repo)) = name.split_once('/') {
        (Some(format!("{}/", ns)), repo.to_string())
    } else {
        (None, name.clone())
    };

    h_flex()
        .id(ElementId::Name(format!("hub-card-{}", pull_ref).into()))
        .w_full()
        .p_3()
        .gap_3()
        .rounded(p.radius.lg)
        .border_1()
        .border_color(p.border.opacity(0.15))
        .bg(p.muted.opacity(0.03))
        .hover(|s| {
            s.bg(p.muted.opacity(0.07))
                .border_color(p.primary.opacity(0.25))
        })
        .items_center()
        .justify_between()
        .child(
            h_flex()
                .flex_1()
                .min_w_0()
                .gap_3()
                .items_center()
                .child(
                    // 状态徽标图标槽
                    v_flex()
                        .w(px(36.0))
                        .h(px(36.0))
                        .rounded(p.radius.md)
                        .bg(if is_official {
                            p.primary.opacity(0.12)
                        } else {
                            p.muted.opacity(0.08)
                        })
                        .items_center()
                        .justify_center()
                        .child(
                            Icon::new(if is_official {
                                IconName::ShieldCheck
                            } else {
                                IconName::Layers
                            })
                            .with_size(px(18.0))
                            .text_color(if is_official {
                                p.primary
                            } else {
                                p.muted_foreground
                            }),
                        ),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_1()
                        .child(
                            h_flex()
                                .gap_2()
                                .items_center()
                                .child(
                                    h_flex()
                                        .items_baseline()
                                        .when_some(namespace_part, |this, ns| {
                                            this.child(
                                                div()
                                                    .text_sm()
                                                    .text_color(p.muted_foreground)
                                                    .child(ns),
                                            )
                                        })
                                        .child(
                                            div()
                                                .text_sm()
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_color(p.foreground)
                                                .child(repo_part),
                                        ),
                                )
                                .when(is_official, |this| {
                                    this.child(
                                        h_flex()
                                            .gap_1()
                                            .items_center()
                                            .px_1p5()
                                            .py_0p5()
                                            .rounded(p.radius.full)
                                            .bg(p.primary.opacity(0.12))
                                            .border_1()
                                            .border_color(p.primary.opacity(0.20))
                                            .child(
                                                Icon::new(IconName::Check)
                                                    .with_size(px(10.0))
                                                    .text_color(p.primary),
                                            )
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .text_color(p.primary)
                                                    .child(
                                                        rust_i18n::t!("container.hub.official")
                                                            .to_string(),
                                                    ),
                                            ),
                                    )
                                })
                                .when(is_automated, |this| {
                                    this.child(
                                        div()
                                            .px_1p5()
                                            .py_0p5()
                                            .text_xs()
                                            .rounded(p.radius.sm)
                                            .bg(p.muted.opacity(0.08))
                                            .text_color(p.muted_foreground)
                                            .child(
                                                rust_i18n::t!("container.hub.automated")
                                                    .to_string(),
                                            ),
                                    )
                                })
                                .child(
                                    h_flex()
                                        .gap_1()
                                        .items_center()
                                        .px_1p5()
                                        .py_0p5()
                                        .rounded(p.radius.full)
                                        .bg(p.warning.opacity(0.08))
                                        .child(
                                            Icon::new(IconName::Star)
                                                .with_size(px(11.0))
                                                .text_color(p.warning),
                                        )
                                        .child(
                                            div()
                                                .text_xs()
                                                .font_weight(FontWeight::MEDIUM)
                                                .text_color(p.warning)
                                                .child(stars_str),
                                        ),
                                ),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(p.muted_foreground)
                                .line_clamp(2)
                                .child(desc),
                        ),
                ),
        )
        .child(
            Button::new(ElementId::Name(format!("hub-pull-{}", pull_ref).into()))
                .secondary()
                .small()
                .icon(IconName::Download)
                .label(rust_i18n::t!("container.hub.pull").to_string())
                .on_click(move |_, window, cx| {
                    on_pull(window, cx);
                }),
        )
        .into_any_element()
}

/// 渲染弹窗底部操作条。
pub(super) fn render_hub_footer<F>(on_close: F, p: &Palette) -> AnyElement
where
    F: Fn(&mut Window, &mut App) + 'static,
{
    h_flex()
        .w_full()
        .justify_between()
        .items_center()
        .pt_2()
        .border_t_1()
        .border_color(p.border.opacity(0.12))
        .child(
            h_flex()
                .gap_1p5()
                .items_center()
                .child(
                    Icon::new(IconName::CircleAlert)
                        .with_size(px(13.0))
                        .text_color(p.muted_foreground),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(rust_i18n::t!("container.hub.footer_hint").to_string()),
                ),
        )
        .child(
            Button::new("container-hub-close")
                .ghost()
                .small()
                .label(rust_i18n::t!("container.hub.close").to_string())
                .on_click(move |_, window, cx| {
                    on_close(window, cx);
                }),
        )
        .into_any_element()
}
