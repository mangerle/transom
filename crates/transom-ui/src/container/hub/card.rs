//! 镜像仓库单个搜索结果卡片视图组件。
//!
//! 设计原理（Why）：
//! - 结构化呈现镜像元数据（官方认证、Star 评级、简要描述）；
//! - 内嵌高频版本标签胶囊选择，支持秒级切换目标版本并直接拉取；
//! - 提供显式的「详情与版本」入口，打通深入查看与按需下载的全链路。

use super::tags::{ImageTagPreset, get_image_tag_presets};
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

/// 镜像搜索结果卡片的渲染参数集合，避免六个平铺参数造成顺序错误。
pub(crate) struct HubResultCardParams<'a, FPull, FSelectTag, FDetail> {
    pub item: &'a ImageSearchResult,
    pub current_selected_tag: &'a str,
    pub on_pull: FPull,
    pub on_select_tag: FSelectTag,
    pub on_open_detail: FDetail,
    pub palette: &'a Palette,
}

/// 渲染单个远端镜像搜索结果卡片。
pub(crate) fn render_hub_result_card<FPull, FSelectTag, FDetail>(
    params: HubResultCardParams<'_, FPull, FSelectTag, FDetail>,
) -> AnyElement
where
    FPull: Fn(&str, &mut Window, &mut App) + 'static + Clone,
    FSelectTag: Fn(&str, &mut Window, &mut App) + 'static + Clone,
    FDetail: Fn(&mut Window, &mut App) + 'static + Clone,
{
    let HubResultCardParams {
        item,
        current_selected_tag,
        on_pull,
        on_select_tag,
        on_open_detail,
        palette: p,
    } = params;
    let name = item.name.clone();
    let desc = if item.description.is_empty() {
        rust_i18n::t!("container.hub.no_description").to_string()
    } else {
        item.description.clone()
    };
    let stars_str = format_star_count(item.star_count);
    let is_official = item.is_official;
    let is_automated = item.is_automated;
    let presets = get_image_tag_presets(&name);
    let (namespace_part, repo_part) = match name.split_once('/') {
        Some((ns, repo)) => (Some(format!("{}/", ns)), repo.to_string()),
        None => (None, name.clone()),
    };
    let active_tag = current_selected_tag.to_string();

    let avatar = image_avatar(is_official, p);
    let name_row = image_name_row(namespace_part, repo_part, p);
    let meta_row = image_meta_row(is_official, is_automated, stars_str, p);
    let tag_chips = tag_chip_row(&name, &presets, &active_tag, on_select_tag);
    let left = card_left_column(CardLeftColumn {
        avatar,
        name_row,
        meta_row,
        desc,
        tag_chips,
        palette: p,
    });
    let actions = card_actions(&name, &active_tag, on_pull, on_open_detail);
    card_frame(&name, left, actions, p)
}

/// 卡片左栏的参数集合，避免六个平铺参数。
struct CardLeftColumn<'a> {
    avatar: AnyElement,
    name_row: AnyElement,
    meta_row: AnyElement,
    desc: String,
    tag_chips: AnyElement,
    palette: &'a Palette,
}

/// 卡片左侧：图标 + 名称元信息 + 描述 + 标签选择。
fn card_left_column(params: CardLeftColumn<'_>) -> AnyElement {
    let p = params.palette;
    let CardLeftColumn {
        avatar,
        name_row,
        meta_row,
        desc,
        tag_chips,
        palette: _,
    } = params;
    h_flex()
        .flex_1()
        .min_w_0()
        .gap_3()
        .items_start()
        .child(avatar)
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_1p5()
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(name_row)
                        .child(meta_row),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .line_clamp(2)
                        .child(desc),
                )
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
                                .child(rust_i18n::t!("container.hub.tag_col_tag").to_string()),
                        )
                        .child(tag_chips),
                ),
        )
        .into_any_element()
}

/// 卡片外框：统一宽度、圆角、悬停态与左右分栏。
fn card_frame(name: &str, left: AnyElement, right: AnyElement, p: &Palette) -> AnyElement {
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
        .child(left)
        .child(right)
        .into_any_element()
}

fn image_avatar(is_official: bool, p: &Palette) -> AnyElement {
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
        )
        .into_any_element()
}

fn image_name_row(namespace: Option<String>, repo: String, p: &Palette) -> AnyElement {
    h_flex()
        .items_baseline()
        .when_some(namespace, |this, ns| {
            this.child(div().text_sm().text_color(p.muted_foreground).child(ns))
        })
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(p.foreground)
                .child(repo),
        )
        .into_any_element()
}

fn image_meta_row(is_official: bool, is_automated: bool, stars: String, p: &Palette) -> AnyElement {
    h_flex()
        .gap_2()
        .items_center()
        .when(is_official, |this| this.child(official_badge(p)))
        .when(is_automated, |this| this.child(automated_badge(p)))
        .child(star_badge(stars, p))
        .into_any_element()
}

fn official_badge(p: &Palette) -> AnyElement {
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
                .child(rust_i18n::t!("container.hub.official").to_string()),
        )
        .into_any_element()
}

fn automated_badge(p: &Palette) -> AnyElement {
    div()
        .px_1p5()
        .py_0p5()
        .text_xs()
        .rounded(p.radius.sm)
        .bg(p.surface.faint)
        .text_color(p.muted_foreground)
        .child(rust_i18n::t!("container.hub.automated").to_string())
        .into_any_element()
}

fn star_badge(stars: String, p: &Palette) -> AnyElement {
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
                .child(stars),
        )
        .into_any_element()
}

fn tag_chip_row<FSelectTag>(
    name: &str,
    presets: &[ImageTagPreset],
    active_tag: &str,
    on_select_tag: FSelectTag,
) -> AnyElement
where
    FSelectTag: Fn(&str, &mut Window, &mut App) + 'static + Clone,
{
    let mut chips = h_flex().gap_1().items_center().flex_wrap();
    for preset in presets.iter().take(4) {
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
        chips = chips.child(chip);
    }
    chips.into_any_element()
}

fn card_actions<FPull, FDetail>(
    name: &str,
    pull_tag: &str,
    on_pull: FPull,
    on_open_detail: FDetail,
) -> AnyElement
where
    FPull: Fn(&str, &mut Window, &mut App) + 'static + Clone,
    FDetail: Fn(&mut Window, &mut App) + 'static + Clone,
{
    let pull_tag = pull_tag.to_string();
    let pull_fn = on_pull.clone();
    let name_owned = name.to_string();
    v_flex()
        .gap_1p5()
        .items_end()
        .child(
            action_button(
                ElementId::Name(format!("hub-pull-btn-{}", name_owned).into()),
                UiIcon::DOWNLOAD,
                format!("{}:{}", rust_i18n::t!("container.hub.pull"), pull_tag),
            )
            .primary()
            .tooltip(format!("docker pull {}:{}", name_owned, pull_tag))
            .on_click(move |_, window, cx| {
                pull_fn(&pull_tag, window, cx);
            }),
        )
        .child(
            action_button(
                ElementId::Name(format!("hub-detail-btn-{}", name_owned).into()),
                UiIcon::RIGHT,
                rust_i18n::t!("container.hub.view_details").to_string(),
            )
            .ghost()
            .on_click(move |_, window, cx| {
                on_open_detail(window, cx);
            }),
        )
        .into_any_element()
}
