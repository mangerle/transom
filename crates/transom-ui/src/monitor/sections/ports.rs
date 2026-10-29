//! 监听端口（Listening Ports）看板。

use crate::app::AppShell;
use crate::monitor::components::CollapsibleCard;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::Sizable;
use gpui_kit::component::input::Input;
use gpui_kit::component::list::ListItem;
use gpui_kit::*;
use transom_core::{PortListeningInfo, SystemStats};

impl AppShell {
    /// 渲染监听端口监控卡片。
    pub(crate) fn render_ports_section_card(
        &self,
        stats: &SystemStats,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let collapsed = self.monitor.collapse.ports;
        let total = stats.ports.len();
        // 采样每轮整体替换 stats，筛选词只存在输入框里，因此刷新不会打断正在输入的条件。
        let query = self.monitor.ports_filter.read(cx).value().to_lowercase();
        let matched: Vec<&PortListeningInfo> = stats
            .ports
            .iter()
            .filter(|port| {
                query.is_empty()
                    || port.port.to_string().contains(&query)
                    || port.protocol.to_lowercase().contains(&query)
                    || port.bind_addr.to_lowercase().contains(&query)
                    || port
                        .process_name
                        .as_ref()
                        .is_some_and(|name| name.to_lowercase().contains(&query))
            })
            .collect();

        let summary = div()
            .text_xs()
            .text_color(p.muted_foreground)
            .child(if query.is_empty() {
                format!("{total} 个监听端口")
            } else {
                format!("命中 {} / 共 {total} 个监听端口", matched.len())
            });

        let mut content = v_flex().gap_1();

        // 输入框常驻在数据有无之前：没采到数据时也留着，免得用户以为是筛选功能坏了。
        if total > 0 {
            content = content.child(Input::new(&self.monitor.ports_filter).small());
        }

        if stats.ports.is_empty() {
            content = content.child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child("暂无监听端口数据或无权限读取"),
            );
        } else if matched.is_empty() {
            content = content.child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(format!("没有匹配 \"{query}\" 的监听端口")),
            );
        } else {
            // 表头
            content = content.child(
                h_flex()
                    .gap_2()
                    .px_3()
                    .py_0p5()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(div().w(px(40.0)).child("协议"))
                    .child(div().w(px(52.0)).child("端口"))
                    .child(div().w(px(80.0)).child("绑定地址"))
                    .child(div().flex_1().child("所属进程")),
            );

            let mut list = v_flex().w_full();
            for (idx, port) in matched.iter().take(30).enumerate() {
                let proc_text = match (&port.process_name, port.pid) {
                    (Some(name), Some(pid)) => format!("{name} ({pid})"),
                    (Some(name), None) => name.clone(),
                    (None, Some(pid)) => format!("PID: {pid}"),
                    (None, None) => "-".to_string(),
                };

                let mut row = ListItem::new(ElementId::Name(format!("port-row-{idx}").into()))
                    .py(px(2.0))
                    .child(
                        h_flex()
                            .w_full()
                            .gap_2()
                            .items_center()
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
                            ),
                    );

                let bg_color = if idx % 2 == 1 {
                    p.list_even
                } else {
                    p.background
                };
                row = row.bg(bg_color);
                list = list.child(row);
            }
            content = content.child(list);
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
