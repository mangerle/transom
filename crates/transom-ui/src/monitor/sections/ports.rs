//! 监听端口（Listening Ports）看板。

use crate::app::AppShell;
use crate::monitor::components::CollapsibleCard;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::*;
use transom_core::SystemStats;

impl AppShell {
    /// 渲染监听端口监控卡片。
    pub(crate) fn render_ports_section_card(
        &self,
        stats: &SystemStats,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let collapsed = self.monitor.collapse.ports;
        let count = stats.ports.len();

        let summary = div()
            .text_xs()
            .text_color(p.muted_foreground)
            .child(format!("{count} 个监听端口"));

        let mut content = v_flex().gap_1();

        if stats.ports.is_empty() {
            content = content.child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child("暂无监听端口数据或无权限读取"),
            );
        } else {
            // 表头
            content = content.child(
                h_flex()
                    .gap_2()
                    .px_1()
                    .py_0p5()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(div().w(px(40.0)).child("协议"))
                    .child(div().w(px(52.0)).child("端口"))
                    .child(div().w(px(80.0)).child("绑定地址"))
                    .child(div().flex_1().child("所属进程")),
            );

            for (idx, port) in stats.ports.iter().take(30).enumerate() {
                let proc_text = match (&port.process_name, port.pid) {
                    (Some(name), Some(pid)) => format!("{name} ({pid})"),
                    (Some(name), None) => name.clone(),
                    (None, Some(pid)) => format!("PID: {pid}"),
                    (None, None) => "-".to_string(),
                };

                let mut row = h_flex()
                    .gap_2()
                    .px_1()
                    .py_0p5()
                    .rounded_xs()
                    .text_xs()
                    .child(
                        div()
                            .w(px(40.0))
                            .text_color(p.primary)
                            .child(port.protocol.to_uppercase()),
                    )
                    .child(
                        div()
                            .w(px(52.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(p.foreground)
                            .child(port.port.to_string()),
                    )
                    .child(
                        div()
                            .w(px(80.0))
                            .truncate()
                            .text_color(p.muted_foreground)
                            .child(port.bind_addr.clone()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .truncate()
                            .text_color(p.foreground)
                            .child(proc_text),
                    );

                if idx % 2 == 1 {
                    row = row.bg(p.muted.opacity(0.3));
                }
                content = content.child(row);
            }
        }

        CollapsibleCard::new("card-ports", IconName::Plug, "监听端口 (Ports)".to_string())
            .collapsed(collapsed)
            .summary(summary)
            .content(content)
            .render(
                p,
                cx.listener(|this, _, _, cx| {
                    this.toggle_section_collapse("ports", cx);
                }),
            )
    }
}
