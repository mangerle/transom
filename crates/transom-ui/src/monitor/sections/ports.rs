//! 监听端口（Listening Ports）看板。

use crate::app::AppShell;
use crate::monitor::components::CollapsibleCard;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{ScrollBounce, h_flex, v_flex, v_virtual_list};
use gpui_kit::component::Sizable;
use gpui_kit::component::input::Input;
use gpui_kit::component::list::ListItem;
use gpui_kit::component::scroll::Scrollbar;
use gpui_kit::*;
use std::rc::Rc;
use transom_core::{PortListeningInfo, SystemStats};

/// 端口列表可视区高度：端口数量中等，留一屏多一点即可，避免挤占上方资源卡片。
const PORTS_LIST_HEIGHT: f32 = 260.0;

/// 端口行高：必须与虚拟列表申报值一致，否则行会互相压盖。
const PORT_ROW_HEIGHT: f32 = 24.0;

/// 端口行的展示数据。
///
/// 虚拟列表闭包要求 `'static`，格式化文本必须在进入闭包前算好并随行数据一起交出。
struct PortRow {
    protocol: String,
    port: u16,
    bind_addr: String,
    process: String,
}

impl AppShell {
    /// 渲染监听端口监控卡片。
    ///
    /// `scope` 区分宿主位置（抽屉 / 全屏标签页）：虚拟列表的滚动状态挂在元素 id 上，
    /// 两处共用同一个 id 会让两个列表的滚动位置互相牵动。
    pub(crate) fn render_ports_section_card(
        &self,
        stats: &SystemStats,
        cx: &mut Context<Self>,
        p: &Palette,
        scope: &str,
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

        // 与进程卡片同一口径：命中数 / 总数，折叠展开都常驻标题栏。
        let summary = div()
            .text_xs()
            .text_color(p.muted_foreground)
            .child(format!("{} / {total}", matched.len()));

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
                    .child(rust_i18n::t!("monitor.ports.empty").to_string()),
            );
        } else if matched.is_empty() {
            content = content.child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("monitor.ports.no_match", query = &query).to_string()),
            );
        } else {
            // 表头：右侧常驻显示命中数 / 总数，展开时也能看到总量。
            content = content.child(
                h_flex().justify_between().items_center().child(
                    h_flex()
                        .gap_2()
                        .px_1()
                        .text_xs()
                        .text_color(p.muted_foreground)
                        .child(
                            div()
                                .w(px(40.0))
                                .child(rust_i18n::t!("monitor.ports.col_protocol").to_string()),
                        )
                        .child(
                            div()
                                .w(px(52.0))
                                .child(rust_i18n::t!("monitor.ports.col_port").to_string()),
                        )
                        .child(
                            div()
                                .w(px(80.0))
                                .child(rust_i18n::t!("monitor.ports.col_bind_addr").to_string()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .child(rust_i18n::t!("monitor.ports.col_process").to_string()),
                        ),
                ),
            );

            let rows: Vec<PortRow> = matched
                .iter()
                .map(|port| PortRow {
                    protocol: port.protocol.to_uppercase(),
                    port: port.port,
                    bind_addr: port.bind_addr.clone(),
                    process: match (&port.process_name, port.pid) {
                        (Some(name), Some(pid)) => format!("{name} ({pid})"),
                        (Some(name), None) => name.clone(),
                        (None, Some(pid)) => format!("PID: {pid}"),
                        (None, None) => "-".to_string(),
                    },
                })
                .collect();
            content = content.child(self.render_ports_list(cx, rows, p, scope));
        }

        CollapsibleCard::new(
            "card-ports",
            IconName::Plug,
            rust_i18n::t!("monitor.ports.title").to_string(),
        )
        .collapsed(collapsed)
        .summary(summary)
        .summary_always(true)
        .content(content)
        .render(
            p,
            cx.listener(|this, _, _, cx| {
                this.toggle_section_collapse("ports", cx);
            }),
        )
    }

    /// 虚拟滚动端口列表：固定视口，只渲染可视行。
    fn render_ports_list(
        &self,
        cx: &mut Context<Self>,
        rows: Vec<PortRow>,
        p: &Palette,
        scope: &str,
    ) -> AnyElement {
        let rows = Rc::new(rows);
        let palette = p.clone();
        let scroll = self.monitor.ports_scroll.clone();
        let list_id = format!("ports-virtual-list-{scope}");
        let item_sizes: Rc<Vec<Size<Pixels>>> =
            Rc::new(vec![
                size(px(PORT_ROW_HEIGHT), px(PORT_ROW_HEIGHT));
                rows.len()
            ]);
        div()
            .relative()
            .w_full()
            .h(px(PORTS_LIST_HEIGHT))
            .child(
                // 虚拟列表自身不接滚轮，滚轮由 ScrollBounce 接管（与进程列表同一组合）。
                ScrollBounce::new(
                    format!("ports-list-scroll-{scope}"),
                    &scroll,
                    v_virtual_list(
                        cx.entity(),
                        ElementId::Name(list_id.into()),
                        item_sizes,
                        move |_, range, _window, _cx| {
                            range
                                .filter_map(|index| {
                                    let row = rows.get(index)?;
                                    Some(Self::render_port_row(index, row, &palette))
                                })
                                .collect()
                        },
                    )
                    .track_scroll(&scroll)
                    .w_full()
                    .h_full(),
                )
                .enabled(false),
            )
            .child(Scrollbar::vertical(&scroll).viewport_from_layout())
            .into_any_element()
    }

    /// 渲染单行端口。
    fn render_port_row(index: usize, row: &PortRow, p: &Palette) -> AnyElement {
        let bg_color = if index % 2 == 1 {
            p.list_even
        } else {
            p.background
        };
        div()
            .w_full()
            .bg(bg_color)
            .child(
                ListItem::new(ElementId::Name(format!("port-row-{index}").into()))
                    // 组件自带的 py/px 会让行高偏离申报值，这里全部改写并锁死高度。
                    .py(px(0.0))
                    .px(px(4.0))
                    .h(px(PORT_ROW_HEIGHT))
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
                                    .child(row.protocol.clone()),
                            )
                            .child(
                                div()
                                    .w(px(52.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(p.foreground)
                                    .child(row.port.to_string()),
                            )
                            .child(
                                div()
                                    .w(px(80.0))
                                    .truncate()
                                    .text_color(p.muted_foreground)
                                    .child(row.bind_addr.clone()),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .truncate()
                                    .text_color(p.foreground)
                                    .child(row.process.clone()),
                            ),
                    ),
            )
            .into_any_element()
    }
}
