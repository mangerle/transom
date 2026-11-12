//! 镜像仓库单个搜索结果卡片视图组件。
//!
//! 设计原理（Why）：
//! - 结构化呈现镜像元数据（官方认证、Star 评级、简要描述）；
//! - 内嵌高频版本标签胶囊选择，支持秒级切换目标版本并直接拉取；
//! - 提供显式的「详情与版本」入口，打通深入查看与按需下载的全链路。

use super::tags::get_image_tag_presets;
use crate::palette::Palette;
use crate::widgets::action_button;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Selectable, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::ImageSearchResult;
use transom_ui_icons::AppIcon;
use transom_ui_icons::AppIconElement;
use transom_ui_icons::UiIcon;

/// 格式化星标数字（如 12500 -> "12.5k"）。
pub(crate) fn format_star_count(stars: u64) -> String {
    if stars >= 1_000_000 {
        format!("{:.1}M", stars as f64 / 1_000_000.0)
    } else if stars >= 1_000 {
        format!("{:.1}k", stars as f64 / 1_000.0)
    } else {
        stars.to_string()
    }
}

/// 渲染单个远端镜像搜索结果卡片。
pub(crate) fn render_hub_result_card<FPull, FSelectTag, FDetail>(
    item: &ImageSearchResult,
    current_selected_tag: &str,
    on_pull: FPull,
    on_select_tag: FSelectTag,
    on_open_detail: FDetail,
    p: &Palette,
) -> AnyElement
where
    FPull: Fn(&str, &mut Window, &mut App) + 'static + Clone,
    FSelectTag: Fn(&str, &mut Window, &mut App) + 'static + Clone,
    FDetail: Fn(&mut Window, &mut App) + 'static + Clone,
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
    let image_name_for_presets = item.name.clone();
    let tag_presets = get_image_tag_presets(&image_name_for_presets);

    // 智能拆分命名空间与镜像名称（如 bitnami/redis）
    let (namespace_part, repo_part) = if let Some((ns, repo)) = name.split_once('/') {
        (Some(format!("{}/", ns)), repo.to_string())
    } else {
        (None, name.clone())
    };

    let active_tag = current_selected_tag.to_string();
    let on_detail_click = on_open_detail.clone();

    // 构造快捷版本标签胶囊
    let mut tag_chips = h_flex().gap_1().items_center().flex_wrap();
    for preset in tag_presets.iter().take(4) {
        let tag_name = preset.tag.to_string();
        let is_selected = active_tag == preset.tag;
        let select_fn = on_select_tag.clone();
        let tag_click_val = tag_name.clone();

        let chip = Button::new(ElementId::Name(
            format!("hub-tag-{}-{}", name, preset.tag).into(),
        ))
        .xsmall()
        .ghost()
        .selected(is_selected)
        .label(tag_name)
        .tooltip(format!("{}: {}", preset.tag, preset.desc))
        .on_click(move |_, window, cx| {
            select_fn(&tag_click_val, window, cx);
        });

        tag_chips = tag_chips.child(chip);
    }

    let pull_tag = active_tag.clone();
    let pull_fn = on_pull.clone();

    h_flex()
        .id(ElementId::Name(format!("hub-card-{}", name).into()))
        .w_full()
        .p_3()
        .gap_3()
        .rounded(p.radius.lg)
        .border_1()
        .border_color(p.stroke.subtle)
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
                .items_start()
                .child(
                    // 状态徽标图标槽
                    v_flex()
                        .w(px(38.0))
                        .h(px(38.0))
                        .rounded(p.radius.md)
                        .bg(if is_official {
                            p.tone.primary_subtle
                        } else {
                            p.surface.faint
                        })
                        .items_center()
                        .justify_center()
                        .child(
                            AppIconElement::new(if is_official {
                                AppIcon::Ui(UiIcon::LOCKED)
                            } else {
                                AppIcon::Ui(UiIcon::DEPENDENCIES)
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
                        .gap_1p5()
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
                                            .bg(p.tone.primary_subtle)
                                            .border_1()
                                            .border_color(p.primary.opacity(0.20))
                                            .child(
                                                AppIconElement::new(AppIcon::Ui(UiIcon::CHECK))
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
                                            .bg(p.surface.faint)
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
                                            AppIconElement::new(AppIcon::Ui(UiIcon::STAR))
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
                        )
                        // 版本标签快捷选择行
                        .child(
                            h_flex()
                                .gap_1p5()
                                .items_center()
                                .pt_0p5()
                                .child(
                                    div()
                                        .text_xs()
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(p.text.subtle)
                                        .child(
                                            rust_i18n::t!("container.hub.tag_col_tag").to_string(),
                                        ),
                                )
                                .child(tag_chips),
                        ),
                ),
        )
        // 右侧动作按钮组
        .child(
            v_flex()
                .gap_1p5()
                .items_end()
                .child(
                    action_button(
                        ElementId::Name(format!("hub-pull-btn-{}", name).into()),
                        UiIcon::DOWNLOAD,
                        format!("{}:{}", rust_i18n::t!("container.hub.pull"), pull_tag),
                    )
                    .primary()
                    .tooltip(format!("docker pull {}:{}", name, pull_tag))
                    .on_click(move |_, window, cx| {
                        pull_fn(&pull_tag, window, cx);
                    }),
                )
                .child(
                    action_button(
                        ElementId::Name(format!("hub-detail-btn-{}", name).into()),
                        UiIcon::RIGHT,
                        rust_i18n::t!("container.hub.view_details").to_string(),
                    )
                    .ghost()
                    .on_click(move |_, window, cx| {
                        on_detail_click(window, cx);
                    }),
                ),
        )
        .into_any_element()
}
