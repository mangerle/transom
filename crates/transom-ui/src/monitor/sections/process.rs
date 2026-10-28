//! 进程管理板块（动态排序、完整命令行展开、终端追踪与防误杀保护）。

use crate::app::AppShell;
use crate::monitor::components::CollapsibleCard;
use crate::monitor::types::{ProcessSort, usage_color};
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{Selectable, h_flex, v_flex};
use gpui_kit::component::button::Button;
use gpui_kit::component::input::Input;
use gpui_kit::component::list::ListItem;
use gpui_kit::component::menu::{ContextMenuExt, PopupMenuItem};
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;
use transom_core::ProcessInfo;

impl AppShell {
    /// 渲染进程管理监控卡片。
    pub(crate) fn render_process_section_card(
        &mut self,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let collapsed = self.monitor.collapse.processes;
        let count = self.monitor.processes.len();

        let summary = div()
            .text_xs()
            .text_color(p.muted_foreground)
            .child(rust_i18n::t!("monitor.process_count", count = count).to_string());

        let filter = self.monitor.filter.read(cx).value().to_lowercase();
        let mut processes: Vec<&ProcessInfo> = self
            .monitor
            .processes
            .iter()
            .filter(|process| {
                filter.is_empty()
                    || process.cmdline.to_lowercase().contains(&filter)
                    || process.user.to_lowercase().contains(&filter)
                    || process.pid.to_string().contains(&filter)
                    || process
                        .args
                        .as_ref()
                        .is_some_and(|a| a.to_lowercase().contains(&filter))
            })
            .collect();

        // 本地视图二次兜底排序
        match self.monitor.sort {
            ProcessSort::Cpu => processes.sort_by(|a, b| {
                b.cpu_percent
                    .partial_cmp(&a.cpu_percent)
                    .unwrap_or(std::cmp::Ordering::Equal)
            }),
            ProcessSort::Memory => processes.sort_by(|a, b| {
                b.mem_percent
                    .partial_cmp(&a.mem_percent)
                    .unwrap_or(std::cmp::Ordering::Equal)
            }),
        }

        let sort = self.monitor.sort;
        let app_entity = cx.entity();
        let expanded_pid = self.monitor.expanded_process_pid;

        let mut content = v_flex().gap_2();

        // 顶层工具栏：排序切换芯片 + 进程搜索框
        content = content
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .child(
                        h_flex()
                            .gap_1()
                            .child(
                                Button::new("sort-process-cpu")
                                    .xsmall()
                                    .label(rust_i18n::t!("monitor.sort_cpu").to_string())
                                    .selected(sort == ProcessSort::Cpu)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.toggle_process_sort(ProcessSort::Cpu, cx);
                                    })),
                            )
                            .child(
                                Button::new("sort-process-mem")
                                    .xsmall()
                                    .label(rust_i18n::t!("monitor.sort_mem").to_string())
                                    .selected(sort == ProcessSort::Memory)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.toggle_process_sort(ProcessSort::Memory, cx);
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.muted_foreground)
                            .child(format!("Top {}", processes.len())),
                    ),
            )
            .child(Input::new(&self.monitor.filter).small())
            .child(
                h_flex()
                    .gap_2()
                    .px_1()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(div().w(px(52.0)).child("PID"))
                    .child(
                        div()
                            .w(px(60.0))
                            .child(rust_i18n::t!("monitor.user").to_string()),
                    )
                    .child(div().w(px(44.0)).child("CPU"))
                    .child(div().w(px(44.0)).child("MEM"))
                    .child(
                        div()
                            .flex_1()
                            .child(rust_i18n::t!("monitor.command").to_string()),
                    ),
            );

        // 进程列表行渲染
        for (index, process) in processes.iter().enumerate() {
            let is_expanded = expanded_pid == Some(process.pid);
            content = content.child(Self::render_process_item(
                index,
                process,
                is_expanded,
                &app_entity,
                p,
            ));
        }

        CollapsibleCard::new(
            "card-processes",
            IconName::ListTree,
            rust_i18n::t!("monitor.processes").to_string(),
        )
        .collapsed(collapsed)
        .summary(summary)
        .content(content)
        .render(
            p,
            cx.listener(|this, _, _, cx| {
                this.toggle_section_collapse("processes", cx);
            }),
        )
    }

    /// 渲染单行进程（支持点击展开/折叠完整命令行参数，右键菜单支持终端追踪与安全杀进程）。
    fn render_process_item(
        index: usize,
        process: &ProcessInfo,
        is_expanded: bool,
        app_entity: &Entity<AppShell>,
        p: &Palette,
    ) -> AnyElement {
        let pid = process.pid;
        let command = process
            .args
            .as_deref()
            .unwrap_or(&process.cmdline)
            .to_string();
        let app = app_entity.clone();
        let term_app = app_entity.clone();
        let safe_kill_app = app_entity.clone();
        let force_kill_app = app_entity.clone();

        let mut row = v_flex().w_full().rounded_xs();

        let list_item = ListItem::new(ElementId::Name(format!("process-row-{pid}").into()))
            .selected(is_expanded)
            .py(px(2.0))
            .on_click(move |_, _, cx| {
                app.update(cx, |this, cx| {
                    if this.monitor.expanded_process_pid == Some(pid) {
                        this.monitor.expanded_process_pid = None;
                    } else {
                        this.monitor.expanded_process_pid = Some(pid);
                    }
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
                            .w(px(52.0))
                            .text_color(p.muted_foreground)
                            .child(pid.to_string()),
                    )
                    .child(
                        div()
                            .w(px(60.0))
                            .truncate()
                            .text_color(p.foreground)
                            .child(process.user.clone()),
                    )
                    .child(
                        div()
                            .w(px(44.0))
                            .text_color(usage_color(process.cpu_percent, p))
                            .child(format!("{:.1}", process.cpu_percent)),
                    )
                    .child(
                        div()
                            .w(px(44.0))
                            .text_color(p.muted_foreground)
                            .child(format!("{:.1}", process.mem_percent)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .truncate()
                            .text_color(p.foreground)
                            .child(process.cmdline.clone()),
                    ),
            );

        let mut main_line = div().w_full().child(list_item).context_menu({
            let command_copy = command.clone();
            move |menu, _window, _cx| {
                let term = term_app.clone();
                let safe_kill = safe_kill_app.clone();
                let force_kill = force_kill_app.clone();
                let cmd_for_clipboard = command_copy.clone();
                menu.item(
                    PopupMenuItem::new("在终端跟踪该进程 (htop/top)".to_string())
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
        });

        if index % 2 == 1 {
            main_line = main_line.bg(p.muted.opacity(0.1));
        }
        row = row.child(main_line);

        // 点击展开完整命令行与启动参数
        if is_expanded {
            row = row.child(
                div()
                    .px_2()
                    .py_1()
                    .bg(p.muted.opacity(0.35))
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
                            .child(div().flex_1().child(command)),
                    ),
            );
        }

        row.into_any_element()
    }
}
