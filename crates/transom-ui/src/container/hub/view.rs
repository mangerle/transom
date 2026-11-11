//! 远端镜像仓库探索弹窗整体布局与容器视图组件。
//!
//! 设计原理（Why）：
//! - 将镜像搜索、版本标签选择与全景详情整合进同一交互弹窗中；
//! - 采用组件分层架构（Card / Detail / Tags），保持各子模块单一职责与单文件行数精简；
//! - 遵循纯组件契约，接收状态与行为闭包，杜绝跨层借用与重入死锁。

use crate::palette::Palette;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::*;
use transom_ui_icons::AppIcon;
use transom_ui_icons::AppIconElement;
use transom_ui_icons::CustomIcon;
use transom_ui_icons::UiIcon;

/// 常用热门探索镜像标签列表。
pub(crate) const POPULAR_IMAGE_TAGS: &[&str] = &[
    "nginx", "redis", "mysql", "postgres", "node", "alpine", "python", "ubuntu", "golang",
];

/// 渲染镜像仓库弹窗顶部 Header。
pub(crate) fn render_hub_header(current_registry_name: &str, p: &Palette) -> AnyElement {
    h_flex()
        .w_full()
        .items_center()
        .justify_between()
        .pb_3()
        .border_b_1()
        .border_color(p.stroke.faint)
        .child(
            h_flex()
                .gap_3()
                .items_center()
                .child(
                    v_flex()
                        .w(px(34.0))
                        .h(px(34.0))
                        .rounded(p.radius.md)
                        .bg(p.tone.primary_subtle)
                        .items_center()
                        .justify_center()
                        .child(
                            AppIconElement::new(AppIcon::Custom(CustomIcon::DOCKER))
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
                .bg(p.surface.faint)
                .border_1()
                .border_color(p.stroke.subtle)
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
pub(crate) fn render_popular_chips<F>(on_select: F, p: &Palette) -> AnyElement
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
                    AppIconElement::new(AppIcon::Ui(UiIcon::AI_INTENTION))
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

/// 渲染初始未搜索时的探索指南面板。
pub(crate) fn render_hub_welcome_guide(p: &Palette) -> AnyElement {
    let guide_card = |icon: AppIcon, icon_color: Hsla, title: String, desc: String| {
        v_flex()
            .flex_1()
            .min_w_0()
            .p_3()
            .gap_2()
            .rounded(p.radius.lg)
            .border_1()
            .border_color(p.stroke.subtle)
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
                            .child(
                                AppIconElement::new(icon)
                                    .with_size(px(14.0))
                                    .text_color(icon_color),
                            ),
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
                            AppIconElement::new(AppIcon::Ui(UiIcon::WEB_SERVER))
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
                    AppIcon::Ui(UiIcon::LOCKED),
                    p.success,
                    rust_i18n::t!("container.hub.guide_official_title").to_string(),
                    rust_i18n::t!("container.hub.guide_official_desc").to_string(),
                ))
                .child(guide_card(
                    AppIcon::Ui(UiIcon::STAR),
                    p.warning,
                    rust_i18n::t!("container.hub.guide_stars_title").to_string(),
                    rust_i18n::t!("container.hub.guide_stars_desc").to_string(),
                ))
                .child(guide_card(
                    AppIcon::Ui(UiIcon::DOWNLOAD),
                    p.primary,
                    rust_i18n::t!("container.hub.guide_pull_title").to_string(),
                    rust_i18n::t!("container.hub.guide_pull_desc").to_string(),
                )),
        )
        .into_any_element()
}

/// 渲染搜索结果列表顶部状态统计条。
pub(crate) fn render_hub_results_header(
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
                    AppIconElement::new(AppIcon::Ui(UiIcon::DEPENDENCIES))
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

/// 渲染弹窗底部操作条。
pub(crate) fn render_hub_footer<F>(on_close: F, p: &Palette) -> AnyElement
where
    F: Fn(&mut Window, &mut App) + 'static,
{
    h_flex()
        .w_full()
        .justify_between()
        .items_center()
        .pt_2()
        .border_t_1()
        .border_color(p.stroke.faint)
        .child(
            h_flex()
                .gap_1p5()
                .items_center()
                .child(
                    AppIconElement::new(AppIcon::Ui(UiIcon::ERROR))
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
