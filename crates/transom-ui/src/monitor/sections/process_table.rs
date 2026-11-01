use super::process::{
    COL_CPU, COL_MEM, COL_PID, COL_USER, PROCESS_LIST_HEIGHT, PROCESS_ROW_HEIGHT, SortColumn,
};
use crate::monitor::MonitorView;
use crate::monitor::types::{ProcessSort, format_size, usage_color};
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{ScrollBounce, h_flex, v_virtual_list};
use gpui_kit::component::list::ListItem;
use gpui_kit::component::menu::{ContextMenuExt, PopupMenu};
use gpui_kit::component::scroll::Scrollbar;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;
use std::rc::Rc;
use transom_core::ProcessInfo;

impl MonitorView {
    /// 渲染可点击排序的表头列：当前排序列显示方向箭头，其余列悬停变亮。
    pub(super) fn render_sort_header(
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
    pub(super) fn render_process_list(
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

        let rows = self.state.processes.clone();
        let palette = p.clone();
        let scroll = self.state.process_scroll.clone();
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
    pub(super) fn render_process_row(
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
        let selected = self.state.selected_process_pid == Some(pid);
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
                    this.state.selected_process_pid =
                        if this.state.selected_process_pid == Some(pid) {
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
                let menu_app = menu_app.clone();
                move |menu, _window, _cx| Self::process_context_menu(menu, &menu_app, pid, &command)
            })
            .into_any_element()
    }

    /// 进程行右键菜单：终端追踪、普通/强制终止与复制完整命令行。
    pub(super) fn process_context_menu(
        menu: PopupMenu,
        app: &Entity<MonitorView>,
        pid: u32,
        command: &str,
    ) -> PopupMenu {
        let command = command.to_string();
        let term = app.clone();
        let safe_kill = app.clone();
        let force_kill = app.clone();
        menu.item(crate::widgets::menu_item(
            rust_i18n::t!("monitor.process_section.trace").to_string(),
            IconName::Terminal,
            move |_, _, cx| {
                term.update(cx, |_this, cx| {
                    cx.emit(crate::monitor::MonitorPanelEvent::TraceProcess(pid));
                });
            },
        ))
        .item(crate::widgets::menu_item(
            rust_i18n::t!("monitor.kill_term").to_string(),
            IconName::CircleX,
            move |_, _, cx| {
                safe_kill.update(cx, |this, cx| {
                    if pid <= 1 {
                        this.set_error(Some(rust_i18n::t!("monitor.error_kill_pid1").to_string()));
                        cx.notify();
                        return;
                    }
                    cx.emit(crate::monitor::MonitorPanelEvent::KillProcess { pid, force: false });
                    cx.notify();
                });
            },
        ))
        .item(crate::widgets::menu_item(
            rust_i18n::t!("monitor.kill_kill").to_string(),
            IconName::Ban,
            move |_, _, cx| {
                force_kill.update(cx, |this, cx| {
                    if pid <= 1 {
                        this.set_error(Some(rust_i18n::t!("monitor.error_kill_pid1").to_string()));
                        cx.notify();
                        return;
                    }
                    cx.emit(crate::monitor::MonitorPanelEvent::KillProcess { pid, force: true });
                    cx.notify();
                });
            },
        ))
        .item(crate::widgets::menu_item(
            rust_i18n::t!("monitor.copy_command").to_string(),
            IconName::Copy,
            move |_, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(command.clone()));
            },
        ))
    }
}
