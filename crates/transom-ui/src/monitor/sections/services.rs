//! Systemd 崩溃服务监控与快速自愈板块。

use crate::monitor::components::CollapsibleCard;
use crate::palette::Palette;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::Button;
use gpui_kit::component::list::ListItem;
use gpui_kit::*;
use transom_core::SystemStats;
use transom_ui_icons::AppIcon;
use transom_ui_icons::AppIconElement;
use transom_ui_icons::CustomIcon;
use transom_ui_icons::UiIcon;

use crate::monitor::MonitorView;

impl MonitorView {
    /// 渲染 Systemd 崩溃服务监控卡片。
    /// 服务板块内容：空态或失败服务列表。
    fn render_services_content(
        &self,
        stats: &SystemStats,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let mut content = v_flex().gap_1p5();

        if stats.failed_services.is_empty() {
            content = content.child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("monitor.services.empty").to_string()),
            );
        } else {
            for (idx, service) in stats.failed_services.iter().enumerate() {
                let unit = service.unit.clone();
                content = content.child(
                    ListItem::new(ElementId::Name(format!("service-item-{idx}").into()))
                        .p_1p5()
                        .rounded_sm()
                        .bg(p.tone.danger_subtle)
                        .border_1()
                        .border_color(p.danger.opacity(0.3))
                        .child(
                            h_flex()
                                .w_full()
                                .justify_between()
                                .items_center()
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
                                        .child(
                                            div().text_xs().text_color(p.muted_foreground).child(
                                                if service.description.is_empty() {
                                                    format!("{}/{}", service.active, service.sub)
                                                } else {
                                                    service.description.clone()
                                                },
                                            ),
                                        ),
                                )
                                .child(
                                    Button::new(format!("restart-service-{}", service.unit))
                                        .xsmall()
                                        .child(
                                            AppIconElement::new(AppIcon::Ui(UiIcon::REFRESH))
                                                .small(),
                                        )
                                        .label(
                                            rust_i18n::t!("monitor.services.restart").to_string(),
                                        )
                                        .on_click(cx.listener(move |_this, _, _, cx| {
                                            cx.emit(
                                                crate::monitor::MonitorPanelEvent::RestartService(
                                                    unit.clone(),
                                                ),
                                            );
                                        })),
                                ),
                        ),
                );
            }
        }

        content.into_any_element()
    }

    pub(crate) fn render_services_section_card(
        &self,
        stats: &SystemStats,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let collapsed = self.state.collapse.services;
        let count = stats.failed_services.len();

        let summary = if count == 0 {
            div()
                .text_xs()
                .text_color(p.sidebar_accent_foreground)
                .child(rust_i18n::t!("monitor.services.healthy").to_string())
        } else {
            div()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(p.danger)
                .child(rust_i18n::t!("monitor.services.failed_count", count = count).to_string())
        };

        let content = self.render_services_content(stats, cx, p);

        CollapsibleCard::new(
            "card-services",
            AppIcon::Custom(CustomIcon::PROCESS),
            rust_i18n::t!("monitor.services.title").to_string(),
        )
        .collapsed(collapsed)
        .summary(summary)
        .content(content)
        .render(
            p,
            cx.listener(|this, _, _, cx| {
                let expanded = this.toggle_collapse("services");
                let collapse = this.collapse_snapshot();
                cx.notify();
                cx.emit(crate::monitor::MonitorPanelEvent::CollapseChanged {
                    collapse,
                    expanded_processes: expanded,
                });
                cx.notify();
            }),
        )
    }
}
