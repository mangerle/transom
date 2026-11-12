//! 健康检查卡片：状态、连续失败次数与最近 5 次检查的垂直时间线。

use super::*;
use crate::container::ContainerWorkspace;
use crate::theme::tokens::typography;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use transom_core::ContainerHealthDetails;
use transom_ui_icons::AppIcon;
use transom_ui_icons::DomainIcon;

/// 健康检查时间线。
///
/// `view` 与其余卡片保持统一的“实体句柄 + Palette”参数形态；当前时间线
/// 没有交互动作，因此只借用不读取。
pub(crate) fn render_health_timeline(
    _view: &Entity<ContainerWorkspace>,
    health: &ContainerHealthDetails,
    p: &Palette,
) -> AnyElement {
    let status_color = if health.status == "healthy" {
        p.success
    } else if health.status == "unhealthy" {
        p.danger
    } else {
        p.warning
    };

    let header_extra = h_flex()
        .gap_2()
        .items_center()
        .child(crate::widgets::status_badge(
            health.status.clone(),
            status_color,
            p,
        ))
        .child(
            div()
                .text_size(rems(typography::CAPTION))
                .text_color(p.muted_foreground)
                .child(format!(
                    "{}: {}",
                    rust_i18n::t!("container.health.failing_streak"),
                    health.failing_streak
                )),
        );

    let latest: Vec<_> = health.log.iter().rev().take(5).collect();
    let mut timeline_items = Vec::with_capacity(latest.len());
    for item in latest {
        let ok = item.exit_code == 0;
        let dot_color = if ok { p.success } else { p.danger };
        let exit_badge =
            crate::widgets::status_badge(format!("exit {}", item.exit_code), dot_color, p);

        let row = h_flex()
            .gap_2()
            .items_start()
            .child(
                v_flex()
                    .items_center()
                    .pt_1()
                    .child(crate::widgets::status_dot_bare(dot_color))
                    .child(div().w(px(1.0)).h(px(16.0)).bg(p.stroke.medium)),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_0p5()
                    .child(
                        h_flex()
                            .justify_between()
                            .items_center()
                            .child(
                                div()
                                    .text_size(rems(typography::CAPTION))
                                    .font_family(p.mono_font_family.clone())
                                    .text_color(p.muted_foreground)
                                    .child(item.start.clone()),
                            )
                            .child(exit_badge),
                    )
                    .when(!item.output.trim().is_empty(), |el| {
                        el.child(
                            div()
                                .p_1p5()
                                .rounded(p.radius.sm)
                                .bg(p.background)
                                .border_1()
                                .border_color(p.border.opacity(0.2))
                                .text_size(rems(typography::CAPTION))
                                .font_family(p.mono_font_family.clone())
                                .text_color(p.foreground.opacity(0.85))
                                .child(item.output.trim().to_string()),
                        )
                    }),
            )
            .into_any_element();
        timeline_items.push(row);
    }

    overview_card(
        format!(
            "{} · {}",
            rust_i18n::t!("container.field.health"),
            health.status
        ),
        AppIcon::Domain(DomainIcon::MONITOR),
        p,
        Some(header_extra.into_any_element()),
        v_flex().gap_1().children(timeline_items),
    )
}
