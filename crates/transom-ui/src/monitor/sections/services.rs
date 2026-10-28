//! Systemd 崩溃服务监控与快速自愈板块。

use crate::app::AppShell;
use crate::monitor::components::CollapsibleCard;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::Button;
use gpui_kit::*;
use transom_core::SystemStats;

impl AppShell {
    /// 渲染 Systemd 崩溃服务监控卡片。
    pub(crate) fn render_services_section_card(
        &self,
        stats: &SystemStats,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let collapsed = self.monitor.collapse.services;
        let count = stats.failed_services.len();

        let summary = if count == 0 {
            div()
                .text_xs()
                .text_color(p.sidebar_accent_foreground)
                .child("服务运行正常")
        } else {
            div()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(p.danger)
                .child(format!("{count} 个异常服务"))
        };

        let mut content = v_flex().gap_1p5();

        if stats.failed_services.is_empty() {
            content = content.child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child("无崩溃服务，所有注册服务运行正常"),
            );
        } else {
            for service in &stats.failed_services {
                let unit = service.unit.clone();
                content = content.child(
                    h_flex()
                        .justify_between()
                        .items_center()
                        .p_1p5()
                        .rounded_sm()
                        .bg(p.danger.opacity(0.12))
                        .border_1()
                        .border_color(p.danger.opacity(0.3))
                        .child(
                            v_flex()
                                .gap_0p5()
                                .child(
                                    div()
                                        .text_xs()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(p.danger)
                                        .child(service.unit.clone()),
                                )
                                .child(div().text_xs().text_color(p.muted_foreground).child(
                                    if service.description.is_empty() {
                                        format!("{}/{}", service.active, service.sub)
                                    } else {
                                        service.description.clone()
                                    },
                                )),
                        )
                        .child(
                            Button::new(format!("restart-service-{}", service.unit))
                                .xsmall()
                                .icon(IconName::RefreshCw)
                                .label("重启")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.restart_systemd_service(&unit, cx);
                                })),
                        ),
                );
            }
        }

        CollapsibleCard::new(
            "card-services",
            IconName::ShieldAlert,
            "服务状态 (Services)".to_string(),
        )
        .collapsed(collapsed)
        .summary(summary)
        .content(content)
        .render(
            p,
            cx.listener(|this, _, _, cx| {
                this.toggle_section_collapse("services", cx);
            }),
        )
    }
}
