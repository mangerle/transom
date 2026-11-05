//! 容器引擎全局存储占用分析图表：四色堆叠占比条与资源分类明细行。
//!
//! 设计原理（Why）：
//! - 将存储分析图表与明细行的渲染逻辑从弹窗外壳（生命周期控制与表单动作）中剥离；
//! - 采用连续分段堆叠条（Stacked Bar）直观展现镜像、容器、卷、构建缓存的相对占比；
//! - 单行明细使用 16px/600 等宽数字并醒目标注可回收容量，消灭原先厚重同构卡片的视觉冗余。

use crate::format::format_size;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;

/// 渲染全局存储占用的四色分段堆叠比例条与图例。
pub(crate) fn render_stacked_usage_bar(
    usage: &transom_core::ContainerSystemDiskUsage,
    p: &Palette,
) -> AnyElement {
    let total = usage.total_size as f32;
    let (img_ratio, cnt_ratio, vol_ratio, bld_ratio) = if total > 0.0 {
        (
            (usage.images_size as f32 / total).clamp(0.0, 1.0),
            (usage.containers_size as f32 / total).clamp(0.0, 1.0),
            (usage.volumes_size as f32 / total).clamp(0.0, 1.0),
            (usage.build_cache_size as f32 / total).clamp(0.0, 1.0),
        )
    } else {
        (0.0, 0.0, 0.0, 0.0)
    };

    let vol_color = p.sidebar_accent_foreground;

    // 堆叠比例条
    let mut bar = h_flex()
        .w_full()
        .h(px(10.0))
        .rounded_full()
        .bg(p.muted.opacity(0.12))
        .overflow_hidden();

    if total > 0.0 {
        if img_ratio > 0.001 {
            bar = bar.child(div().flex_grow(img_ratio).h_full().bg(p.primary));
        }
        if cnt_ratio > 0.001 {
            bar = bar.child(div().flex_grow(cnt_ratio).h_full().bg(p.success));
        }
        if vol_ratio > 0.001 {
            bar = bar.child(div().flex_grow(vol_ratio).h_full().bg(vol_color));
        }
        if bld_ratio > 0.001 {
            bar = bar.child(div().flex_grow(bld_ratio).h_full().bg(p.warning));
        }
    }

    // 图例标签
    let legend = h_flex()
        .gap_3()
        .items_center()
        .justify_between()
        .w_full()
        .text_xs()
        .child(render_legend_item(
            rust_i18n::t!("container.system_df.category_images").to_string(),
            (img_ratio * 100.0).round() as u32,
            p.primary,
            p,
        ))
        .child(render_legend_item(
            rust_i18n::t!("container.system_df.category_containers").to_string(),
            (cnt_ratio * 100.0).round() as u32,
            p.success,
            p,
        ))
        .child(render_legend_item(
            rust_i18n::t!("container.system_df.category_volumes").to_string(),
            (vol_ratio * 100.0).round() as u32,
            vol_color,
            p,
        ))
        .child(render_legend_item(
            rust_i18n::t!("container.system_df.category_build_cache").to_string(),
            (bld_ratio * 100.0).round() as u32,
            p.warning,
            p,
        ));

    v_flex()
        .gap_2()
        .w_full()
        .p_3()
        .rounded(p.radius.md)
        .bg(p.muted.opacity(0.04))
        .border_1()
        .border_color(p.border.opacity(0.4))
        .child(bar)
        .child(legend)
        .into_any_element()
}

fn render_legend_item(label: String, percent: u32, color: Hsla, p: &Palette) -> AnyElement {
    h_flex()
        .gap_1p5()
        .items_center()
        .child(div().size_2().rounded_full().bg(color))
        .child(div().text_color(p.muted_foreground).child(label))
        .child(
            div()
                .font_weight(FontWeight::MEDIUM)
                .text_color(p.foreground)
                .child(rust_i18n::t!("container.system_df.legend_ratio", percent = percent).to_string()),
        )
        .into_any_element()
}

/// 渲染 4 类资源的细致明细行。
pub(crate) fn render_df_details_list(
    usage: &transom_core::ContainerSystemDiskUsage,
    p: &Palette,
) -> AnyElement {
    let vol_color = p.sidebar_accent_foreground;
    v_flex()
        .gap_1p5()
        .w_full()
        .child(render_df_detail_row(
            rust_i18n::t!("container.system_df.category_images").to_string(),
            IconName::Disc,
            usage.images_count,
            usage.images_size,
            usage.images_reclaimable,
            p.primary,
            p,
        ))
        .child(render_df_detail_row(
            rust_i18n::t!("container.system_df.category_containers").to_string(),
            IconName::Box,
            usage.containers_count,
            usage.containers_size,
            usage.containers_reclaimable,
            p.success,
            p,
        ))
        .child(render_df_detail_row(
            rust_i18n::t!("container.system_df.category_volumes").to_string(),
            IconName::Database,
            usage.volumes_count,
            usage.volumes_size,
            usage.volumes_reclaimable,
            vol_color,
            p,
        ))
        .child(render_df_detail_row(
            rust_i18n::t!("container.system_df.category_build_cache").to_string(),
            IconName::Layers,
            usage.build_cache_count,
            usage.build_cache_size,
            usage.build_cache_reclaimable,
            p.warning,
            p,
        ))
        .into_any_element()
}

fn render_df_detail_row(
    title: String,
    icon: IconName,
    count: usize,
    size: u64,
    reclaimable: u64,
    accent_color: Hsla,
    p: &Palette,
) -> AnyElement {
    let size_str = format_size(size);
    let reclaim_str = format_size(reclaimable);

    h_flex()
        .justify_between()
        .items_center()
        .px_3()
        .py_2()
        .rounded(p.radius.md)
        .bg(p.list_background)
        .border_1()
        .border_color(p.border.opacity(0.3))
        .child(
            h_flex()
                .gap_2p5()
                .items_center()
                .child(
                    div()
                        .w(px(3.0))
                        .h(px(16.0))
                        .rounded_full()
                        .bg(accent_color),
                )
                .child(Icon::new(icon).small().text_color(accent_color))
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .child(title),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(format!("({count})")),
                ),
        )
        .child(
            h_flex()
                .gap_3()
                .items_center()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::BOLD)
                        .text_color(p.foreground)
                        .child(size_str),
                )
                .child(if reclaimable > 0 {
                    div()
                        .text_xs()
                        .px_2()
                        .py_0p5()
                        .rounded(p.radius.full)
                        .bg(p.warning.opacity(0.12))
                        .text_color(p.warning)
                        .font_weight(FontWeight::MEDIUM)
                        .child(format!(
                            "{} {reclaim_str}",
                            rust_i18n::t!("container.system_df.reclaimable")
                        ))
                } else {
                    div()
                        .text_xs()
                        .text_color(p.muted_foreground.opacity(0.7))
                        .child(rust_i18n::t!("container.system_df.optimal").to_string())
                }),
        )
        .into_any_element()
}
