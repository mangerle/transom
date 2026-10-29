//! 进程管理板块（虚拟滚动列表、完整命令行详情条、终端追踪与防误杀保护）。

use crate::app::AppShell;
use crate::monitor::components::CollapsibleCard;
use crate::monitor::types::{ProcessSort, format_size, usage_color};
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{ScrollBounce, h_flex, v_flex, v_virtual_list};
use gpui_kit::component::input::Input;
use gpui_kit::component::list::ListItem;
use gpui_kit::component::menu::{ContextMenuExt, PopupMenuItem};
use gpui_kit::component::scroll::Scrollbar;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;
use std::rc::Rc;
use transom_core::ProcessInfo;

/// 进程列表可视区高度。
///
/// 固定高度是虚拟滚动生效的前提：父容器高度不确定时列表会按内容全量展开，
/// 几百个进程就会一次性全部渲染。
const PROCESS_LIST_HEIGHT: f32 = 320.0;

/// 进程行高：虚拟列表按申报高度定位行，行内锁死同样高度才不会错位。
/// 20px 偏挤，取 24px 与组件库列表行的常规留白一致。
const PROCESS_ROW_HEIGHT: f32 = 24.0;

/// 可排序表头列的静态描述，避免渲染函数参数膨胀。
struct SortColumn {
    /// 宿主位置（抽屉 / 全屏标签页），用于生成唯一元素 id。
    scope: &'static str,
    label: String,
    width: f32,
    field: ProcessSort,
}

/// 进程表列宽：表头与数据行共用，保证两行列对齐。
const COL_PID: f32 = 52.0;
const COL_USER: f32 = 60.0;
const COL_CPU: f32 = 44.0;
const COL_MEM: f32 = 88.0;

impl AppShell {
    /// 渲染进程管理监控卡片。
    ///
    /// `scope` 区分宿主位置（抽屉 / 全屏标签页）：虚拟列表的滚动状态挂在元素 id 上，
    /// 两处共用同一个 id 会让两个列表的滚动位置互相牵动。
    pub(crate) fn render_process_section_card(
        &mut self,
        cx: &mut Context<Self>,
        p: &Palette,
        scope: &'static str,
    ) -> AnyElement {
        let collapsed = self.monitor.collapse.processes;
        let total = self.monitor.process_total;
        let sort = self.monitor.sort;
        let ascending = self.monitor.sort_ascending;

        // 过滤与排序结果同时决定摘要计数与虚拟列表条数，二者必须同源。
        let order = self.filtered_order(cx);
        // 折叠态不采样明细，匹配数按总数兜底，摘要才不会显示成 0。
        let matched = if collapsed {
            total
        } else {
            u32::try_from(order.len()).unwrap_or(u32::MAX)
        };

        let summary = div()
            .text_xs()
            .text_color(p.muted_foreground)
            .child(format!("{matched} / {total}"));

        let mut content = v_flex()
            .gap_2()
            .child(Input::new(&self.monitor.filter).small())
            .child(
                h_flex()
                    .gap_2()
                    .px_1()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(
                        div()
                            .w(px(COL_PID))
                            .child(rust_i18n::t!("monitor.col_pid").to_string()),
                    )
                    .child(
                        div()
                            .w(px(COL_USER))
                            .child(rust_i18n::t!("monitor.user").to_string()),
                    )
                    .child(Self::render_sort_header(
                        SortColumn {
                            scope,
                            label: rust_i18n::t!("monitor.col_cpu").to_string(),
                            width: COL_CPU,
                            field: ProcessSort::Cpu,
                        },
                        sort,
                        ascending,
                        p,
                        cx,
                    ))
                    .child(Self::render_sort_header(
                        SortColumn {
                            scope,
                            label: rust_i18n::t!("monitor.col_mem").to_string(),
                            width: COL_MEM,
                            field: ProcessSort::Memory,
                        },
                        sort,
                        ascending,
                        p,
                        cx,
                    ))
                    .child(
                        div()
                            .flex_1()
                            .child(rust_i18n::t!("monitor.command").to_string()),
                    ),
            )
            .child(self.render_process_list(cx, order, p, scope));

        // 选中行后在卡片底部展示完整命令行：行内展开会破坏等高行，撑乱虚拟滚动。
        if let Some(detail) = self.render_process_detail(p) {
            content = content.child(detail);
        }

        CollapsibleCard::new(
            "card-processes",
            IconName::ListTree,
            rust_i18n::t!("monitor.processes").to_string(),
        )
        .collapsed(collapsed)
        .summary(summary)
        .summary_always(true)
        .content(content)
        .render(
            p,
            cx.listener(|this, _, _, cx| {
                this.toggle_section_collapse("processes", cx);
            }),
        )
    }

    /// 过滤与排序后的行下标。
    ///
    /// 表头计数与虚拟列表数据都走这里，保证条数与实际行数不会错位。
    fn filtered_order(&self, cx: &mut Context<Self>) -> Vec<usize> {
        let filter = self.monitor.filter.read(cx).value().to_lowercase();
        let processes = &self.monitor.processes;
        let mut order: Vec<usize> = processes
            .iter()
            .enumerate()
            .filter(|(_, process)| {
                filter.is_empty()
                    || process.cmdline.to_lowercase().contains(&filter)
                    || process.user.to_lowercase().contains(&filter)
                    || process.pid.to_string().contains(&filter)
                    || process
                        .args
                        .as_ref()
                        .is_some_and(|args| args.to_lowercase().contains(&filter))
            })
            .map(|(index, _)| index)
            .collect();
        let ascending = self.monitor.sort_ascending;
        match self.monitor.sort {
            ProcessSort::Cpu => order.sort_by(|a, b| {
                let cmp = processes[*a]
                    .cpu_percent
                    .total_cmp(&processes[*b].cpu_percent);
                if ascending { cmp } else { cmp.reverse() }
            }),
            ProcessSort::Memory => order.sort_by(|a, b| {
                let cmp = processes[*a].rss_bytes.cmp(&processes[*b].rss_bytes);
                if ascending { cmp } else { cmp.reverse() }
            }),
        }
        order
    }

    /// 渲染可点击排序的表头列：当前排序列显示方向箭头，其余列悬停变亮。
    fn render_sort_header(
        column: SortColumn,
        current: ProcessSort,
        ascending: bool,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let SortColumn {
            scope,
            label,
            width,
            field,
        } = column;
        let active = current == field;
        let color = if active {
            p.foreground
        } else {
            p.muted_foreground
        };
        let arrow = if ascending {
            IconName::ChevronUp
        } else {
            IconName::ChevronDown
        };
        let mut header = h_flex().gap_0p5().items_center().child(label.to_string());
        if active {
            header = header.child(Icon::new(arrow).xsmall().text_color(color));
        }
        div()
            .id(ElementId::Name(
                format!("process-sort-{scope}-{label}").into(),
            ))
            .w(px(width))
            .h(px(20.0))
            .cursor_pointer()
            .text_color(color)
            .hover(|style| style.text_color(p.foreground))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.toggle_process_sort(field, cx);
            }))
            .child(header)
            .into_any_element()
    }

    /// 虚拟滚动进程列表：只渲染可视区内的行。
    fn render_process_list(
        &mut self,
        cx: &mut Context<Self>,
        order: Vec<usize>,
        p: &Palette,
        scope: &'static str,
    ) -> AnyElement {
        if order.is_empty() {
            // 没有可渲染的行时给出占位提示：空白区域分不清是"没数据"还是"没渲染出来"。
            return div()
                .id("process-list")
                .w_full()
                .h(px(PROCESS_LIST_HEIGHT))
                .items_center()
                .justify_center()
                .text_xs()
                .text_color(p.muted_foreground)
                .child(rust_i18n::t!("monitor.no_data").to_string())
                .into_any_element();
        }

        let rows = self.monitor.processes.clone();
        let palette = p.clone();
        let scroll = self.monitor.process_scroll.clone();
        let list_id = format!("process-virtual-list-{scope}");
        // 申报高度必须与行内实际高度一致：虚拟列表按这里定位每一行，
        // 一旦偏小，行会互相压盖，表现就是"列表钉死、滚不动"。
        let item_sizes: Rc<Vec<Size<Pixels>>> =
            Rc::new(vec![
                size(px(PROCESS_ROW_HEIGHT), px(PROCESS_ROW_HEIGHT));
                order.len()
            ]);
        div()
            .id("process-list-frame")
            .relative()
            .w_full()
            .h(px(PROCESS_LIST_HEIGHT))
            .child(
                // 虚拟列表自身不接滚轮，滚轮由 ScrollBounce 接管——
                // 组件库的 List 也是这么组合的，缺了它滚轮完全没反应。
                ScrollBounce::new(
                    format!("process-list-scroll-{scope}"),
                    &scroll,
                    v_virtual_list(
                        cx.entity(),
                        ElementId::Name(list_id.into()),
                        item_sizes,
                        move |this, range, _window, cx| {
                            range
                                .filter_map(|index| {
                                    let row = *order.get(index)?;
                                    let process = rows.get(row)?;
                                    Some(this.render_process_row(index, process, &palette, cx))
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

    /// 渲染单行进程：等高行，点击选中后在卡片底部展示完整命令行。
    fn render_process_row(
        &mut self,
        index: usize,
        process: &ProcessInfo,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let pid = process.pid;
        let command = process
            .args
            .as_deref()
            .unwrap_or(&process.cmdline)
            .to_string();
        let selected = self.monitor.selected_process_pid == Some(pid);
        let click_app = cx.entity();
        let menu_app = cx.entity();

        let list_item = ListItem::new(ElementId::Name(format!("process-row-{pid}").into()))
            .selected(selected)
            // 组件自带的 py/px 会让行高偏离申报值，这里全部改写并锁死高度。
            .py(px(0.0))
            .px(px(4.0))
            .h(px(PROCESS_ROW_HEIGHT))
            .on_click(move |_, _, cx| {
                click_app.update(cx, |this, cx| {
                    this.monitor.selected_process_pid =
                        if this.monitor.selected_process_pid == Some(pid) {
                            None
                        } else {
                            Some(pid)
                        };
                    cx.notify();
                });
            })
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .items_center()
                    .text_xs()
                    .child(
                        div()
                            .w(px(COL_PID))
                            .text_color(p.muted_foreground)
                            .child(pid.to_string()),
                    )
                    .child(
                        div()
                            .w(px(COL_USER))
                            .truncate()
                            .text_color(p.foreground)
                            .child(process.user.clone()),
                    )
                    .child(
                        div()
                            .w(px(COL_CPU))
                            .text_color(usage_color(process.cpu_percent, p))
                            .child(format!("{:.1}", process.cpu_percent)),
                    )
                    .child(
                        div().w(px(COL_MEM)).child(
                            h_flex()
                                .gap_1()
                                .text_color(p.foreground)
                                .child(format_size(process.rss_bytes))
                                .child(
                                    div()
                                        .text_color(p.muted_foreground)
                                        .child(format!("({:.1}%)", process.mem_percent)),
                                ),
                        ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .truncate()
                            .text_color(p.foreground)
                            .child(process.cmdline.clone()),
                    ),
            );

        let bg_color = if index % 2 == 1 {
            p.list_even
        } else {
            p.background
        };

        div()
            .w_full()
            .bg(bg_color)
            .child(list_item)
            .context_menu({
                let command = command.clone();
                move |menu, _window, _cx| {
                    let term = menu_app.clone();
                    let safe_kill = menu_app.clone();
                    let force_kill = menu_app.clone();
                    let cmd_for_clipboard = command.clone();
                    menu.item(
                        PopupMenuItem::new(
                            rust_i18n::t!("monitor.process_section.trace").to_string(),
                        )
                        .icon(IconName::Terminal)
                        .on_click(move |_, _, cx| {
                            term.update(cx, |this, cx| {
                                this.trace_process_in_terminal(pid, cx);
                            });
                        }),
                    )
                    .item(
                        PopupMenuItem::new(rust_i18n::t!("monitor.kill_term").to_string())
                            .icon(IconName::CircleX)
                            .on_click(move |_, _, cx| {
                                safe_kill.update(cx, |this, cx| {
                                    this.send_kill_process_safe(pid, false, cx);
                                });
                            }),
                    )
                    .item(
                        PopupMenuItem::new(rust_i18n::t!("monitor.kill_kill").to_string())
                            .icon(IconName::Ban)
                            .on_click(move |_, _, cx| {
                                force_kill.update(cx, |this, cx| {
                                    this.send_kill_process_safe(pid, true, cx);
                                });
                            }),
                    )
                    .item(
                        PopupMenuItem::new(rust_i18n::t!("monitor.copy_command").to_string())
                            .icon(IconName::Copy)
                            .on_click(move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(
                                    cmd_for_clipboard.clone(),
                                ));
                            }),
                    )
                }
            })
            .into_any_element()
    }

    /// 底部详情条：展示选中进程的完整命令行。
    fn render_process_detail(&mut self, p: &Palette) -> Option<AnyElement> {
        let pid = self.monitor.selected_process_pid?;
        let process = self
            .monitor
            .processes
            .iter()
            .find(|process| process.pid == pid)?;
        let command = process.args.as_deref().unwrap_or(&process.cmdline);
        Some(
            div()
                .px_2()
                .py_1()
                .bg(p.muted)
                .rounded_b_xs()
                .border_l_2()
                .border_color(p.primary)
                .text_xs()
                .text_color(p.muted_foreground)
                .child(
                    h_flex()
                        .gap_1()
                        .items_start()
                        .child(Icon::new(IconName::Terminal).xsmall().text_color(p.primary))
                        .child(div().flex_1().child(command.to_string())),
                )
                .into_any_element(),
        )
    }
}
