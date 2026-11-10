//! 镜像详情与全部版本标签（Tags）视图组件。
//!
//! 设计原理（Why）：
//! - 提供独立的全景详情视图，彻底消除“只能下载默认版本、无法看详情”的限制；
//! - 完整呈现镜像描述、官方认证属性、支持架构及常用稳定版标签；
//! - 包含直接输入任意自定义 Tag 的拉取控件与规范 Docker CLI 命令一键复制。

use super::card::format_star_count;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use transom_core::ImageSearchResult;

/// 镜像详情视图的数据参数。
pub(crate) struct HubImageDetailParams<'a> {
    pub(crate) item: &'a ImageSearchResult,
    pub(crate) active_tag: &'a str,
    pub(crate) pull_reference: &'a str,
    pub(crate) tags: &'a [String],
    pub(crate) tags_loading: bool,
    pub(crate) tags_error: Option<&'a str>,
    pub(crate) custom_tag_input: Option<&'a Entity<gpui_kit::component::input::InputState>>,
    pub(crate) copy_toast: bool,
}

/// 渲染镜像全貌详情与版本标签视图。
pub(crate) fn render_hub_image_detail<FBack, FPull, FCopy>(
    params: HubImageDetailParams<'_>,
    on_back: FBack,
    on_pull: FPull,
    on_copy_cmd: FCopy,
    p: &Palette,
) -> AnyElement
where
    FBack: Fn(&mut Window, &mut App) + 'static + Clone,
    FPull: Fn(&str, &mut Window, &mut App) + 'static + Clone,
    FCopy: Fn(&str, &mut Window, &mut App) + 'static + Clone,
{
    let HubImageDetailParams {
        item,
        active_tag,
        pull_reference,
        tags,
        tags_loading,
        tags_error,
        custom_tag_input,
        copy_toast,
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
    let current_tag = active_tag.to_string();

    let cli_cmd = format!("docker pull {pull_reference}");
    let cmd_for_copy = cli_cmd.clone();
    let on_copy = on_copy_cmd.clone();

    // 1. 顶部返回导航栏
    let nav_bar = h_flex()
        .w_full()
        .justify_between()
        .items_center()
        .pb_2()
        .border_b_1()
        .border_color(p.stroke.faint)
        .child(
            Button::new("hub-detail-back-btn")
                .ghost()
                .small()
                .icon(IconName::ArrowLeft)
                .label(rust_i18n::t!("container.hub.back_to_results").to_string())
                .on_click(move |_, window, cx| {
                    on_back(window, cx);
                }),
        )
        .child(
            h_flex()
                .gap_1p5()
                .items_center()
                .child(
                    div()
                        .text_xs()
                        .font_family(p.mono_font_family.clone())
                        .text_color(p.muted_foreground)
                        .child(cli_cmd),
                )
                .child(
                    Button::new("hub-detail-copy-cmd-btn")
                        .ghost()
                        .xsmall()
                        .icon(if copy_toast {
                            IconName::Check
                        } else {
                            IconName::Copy
                        })
                        .label(if copy_toast {
                            rust_i18n::t!("container.hub.copied").to_string()
                        } else {
                            rust_i18n::t!("container.hub.copy_cli").to_string()
                        })
                        .on_click(move |_, window, cx| {
                            on_copy(&cmd_for_copy, window, cx);
                        }),
                ),
        );

    // 2. 镜像核心元数据 Header
    let header_card = h_flex()
        .w_full()
        .gap_3()
        .p_3()
        .rounded(p.radius.lg)
        .bg(p.surface.ghost)
        .border_1()
        .border_color(p.stroke.subtle)
        .items_center()
        .child(
            v_flex()
                .w(px(46.0))
                .h(px(46.0))
                .rounded(p.radius.md)
                .bg(if is_official {
                    p.tone.primary_subtle
                } else {
                    p.surface.faint
                })
                .items_center()
                .justify_center()
                .child(
                    Icon::new(if is_official {
                        IconName::ShieldCheck
                    } else {
                        IconName::Layers
                    })
                    .with_size(px(22.0))
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
                            div()
                                .text_base()
                                .font_weight(FontWeight::BOLD)
                                .text_color(p.foreground)
                                .child(name.clone()),
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
                                                rust_i18n::t!("container.hub.official").to_string(),
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
                                    .child(rust_i18n::t!("container.hub.automated").to_string()),
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
                .child(div().text_xs().text_color(p.muted_foreground).child(desc)),
        );

    // 3. 自定义版本标签快速拉取栏
    let custom_pull_fn = on_pull.clone();
    let custom_tag_row = if let Some(input) = custom_tag_input {
        let input_ent = input.clone();
        h_flex()
            .w_full()
            .gap_2()
            .p_2p5()
            .rounded(p.radius.md)
            .bg(p.surface.ghost)
            .border_1()
            .border_color(p.stroke.subtle)
            .items_center()
            .child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(p.foreground)
                    .child(rust_i18n::t!("container.hub.custom_tag").to_string()),
            )
            .child(h_flex().flex_1().child(Input::new(input).small()))
            .child(
                Button::new("hub-detail-pull-custom-btn")
                    .primary()
                    .small()
                    .icon(IconName::Download)
                    .label(rust_i18n::t!("container.hub.pull_selected").to_string())
                    .on_click(move |_, window, cx| {
                        let custom_val = input_ent.read(cx).value().trim().to_string();
                        let tag_to_pull = if custom_val.is_empty() {
                            "latest".to_string()
                        } else {
                            custom_val
                        };
                        custom_pull_fn(&tag_to_pull, window, cx);
                    }),
            )
            .into_any_element()
    } else {
        div().into_any_element()
    };

    // 4. 常用版本标签列表 (Tags Table)
    let mut tags_list = v_flex().gap_1p5().w_full();

    // 列表表头
    tags_list = tags_list.child(
        h_flex()
            .w_full()
            .px_2()
            .py_1()
            .items_center()
            .text_xs()
            .font_weight(FontWeight::MEDIUM)
            .text_color(p.muted_foreground)
            .child(
                div()
                    .w(px(140.0))
                    .child(rust_i18n::t!("container.hub.tag_col_tag").to_string()),
            )
            .child(
                div()
                    .flex_1()
                    .child(rust_i18n::t!("container.hub.tag_col_desc").to_string()),
            )
            .child(
                div()
                    .w(px(160.0))
                    .child(rust_i18n::t!("container.hub.tag_col_arch").to_string()),
            )
            .child(
                div()
                    .w(px(80.0))
                    .text_right()
                    .child(rust_i18n::t!("container.hub.tag_col_action").to_string()),
            ),
    );

    // 渲染各版本行
    if tags_loading {
        tags_list = tags_list.child(
            h_flex()
                .w_full()
                .p_3()
                .items_center()
                .gap_2()
                .child(
                    Icon::new(IconName::Loader)
                        .xsmall()
                        .text_color(p.muted_foreground),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(rust_i18n::t!("container.hub.tags_loading").to_string()),
                ),
        );
    } else if let Some(error) = tags_error {
        tags_list = tags_list.child(
            div()
                .w_full()
                .p_3()
                .rounded(p.radius.sm)
                .bg(p.danger.opacity(0.06))
                .text_xs()
                .text_color(p.danger)
                .child(error.to_string()),
        );
    } else if tags.is_empty() {
        tags_list = tags_list.child(
            div()
                .w_full()
                .p_3()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("container.hub.tags_empty").to_string()),
        );
    } else {
        for tag in tags {
            let tag_name = tag.clone();
            let is_current = current_tag == *tag;
            let row_pull_fn = on_pull.clone();
            let pull_val = tag_name.clone();

            let row = h_flex()
                .w_full()
                .px_2()
                .py_2()
                .rounded(p.radius.sm)
                .bg(if is_current {
                    p.primary.opacity(0.08)
                } else {
                    p.muted.opacity(0.02)
                })
                .hover(|s| s.bg(p.muted.opacity(0.06)))
                .border_1()
                .border_color(if is_current {
                    p.primary.opacity(0.25)
                } else {
                    p.border.opacity(0.10)
                })
                .items_center()
                .child(
                    div()
                        .w(px(180.0))
                        .text_xs()
                        .font_family(p.mono_font_family.clone())
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(p.foreground)
                        .child(tag_name.clone()),
                )
                .child(
                    div()
                        .flex_1()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(rust_i18n::t!("container.hub.tags_source").to_string()),
                )
                .child(
                    div()
                        .w(px(140.0))
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child("—"),
                )
                .child(
                    h_flex().w(px(80.0)).justify_end().child(
                        Button::new(ElementId::Name(format!("hub-tag-row-pull-{}", tag).into()))
                            .secondary()
                            .xsmall()
                            .icon(IconName::Download)
                            .label(rust_i18n::t!("container.hub.pull").to_string())
                            .on_click(move |_, window, cx| {
                                row_pull_fn(&pull_val, window, cx);
                            }),
                    ),
                );

            tags_list = tags_list.child(row);
        }
    }

    v_flex()
        .w_full()
        .gap_3()
        .child(nav_bar)
        .child(header_card)
        .child(custom_tag_row)
        .child(
            v_flex()
                .w_full()
                .gap_1p5()
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(p.foreground)
                        .child(rust_i18n::t!("container.hub.tags_title").to_string()),
                )
                .child(tags_list),
        )
        .into_any_element()
}
