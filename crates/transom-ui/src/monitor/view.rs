use super::types::{ProcessSort, format_size, usage_bar, usage_color};
use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{Selectable, h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::{ContextMenuExt, PopupMenuItem};
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;
use transom_core::{ProcessInfo, SystemStats};

impl AppShell {
    /// 渲染右侧监控抽屉。
    pub(crate) fn render_monitor_panel(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let stats = self
            .workspace
            .active_tab
            .and_then(|tab_id| self.workspace.tabs.iter().find(|tab| tab.id == tab_id))
            .and_then(|tab| tab.system_stats().cloned());
        let mut body = v_flex().gap_4().p_3();
        if let Some(stats) = &stats {
            body = body
                .child(self.render_cpu_section(stats, p))
                .child(self.render_memory_section(stats, p))
                .child(self.render_filesystems_section(stats, p))
                .child(self.render_network_section(p));
        } else {
            body = body.child(
                div()
                    .text_sm()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("monitor.waiting").to_string()),
            );
        }
        body = body.child(self.render_process_section(cx, p));

        let mut panel = v_flex()
            .id("monitor-panel")
            .size_full()
            .bg(p.background)
            .border_l_1()
            .border_color(p.border);
        panel = panel.child(
            h_flex()
                .justify_between()
                .items_center()
                .px_3()
                .py_2()
                .border_b_1()
                .border_color(p.border)
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(Icon::new(IconName::Gauge).small().text_color(p.primary))
                        .child(
                            div()
                                .text_sm()
                                .child(rust_i18n::t!("monitor.title").to_string()),
                        ),
                )
                .child(
                    h_flex()
                        .gap_1()
                        .child(
                            Button::new("monitor-refresh")
                                .ghost()
                                .small()
                                .icon(IconName::RefreshCw)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.sample_processes(cx);
                                })),
                        )
                        .child(
                            Button::new("monitor-close")
                                .ghost()
                                .small()
                                .icon(IconName::Close)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.monitor.open = false;
                                    this.monitor.processes.clear();
                                    cx.notify();
                                })),
                        ),
                ),
        );
        if let Some(error) = &self.monitor.error {
            panel = panel.child(
                div()
                    .px_3()
                    .py_2()
                    .text_xs()
                    .text_color(p.danger)
                    .child(error.clone()),
            );
        }
        panel
            .child(
                div()
                    .id("monitor-body")
                    .flex_1()
                    .overflow_y_scroll()
                    .child(body),
            )
            .into_any_element()
    }

    fn render_cpu_section(&self, stats: &SystemStats, p: &Palette) -> AnyElement {
        let color = usage_color(stats.cpu_percent, p);
        let mut section = v_flex()
            .gap_2()
            .child(self.metric_header(
                "monitor.cpu",
                format!("{:.0}%", stats.cpu_percent),
                color,
                p,
            ))
            .child(usage_bar(stats.cpu_percent / 100.0, color, p));
        if !stats.cpu_cores.is_empty() {
            section = section.child(h_flex().h(px(42.0)).items_end().gap_0p5().children(
                stats.cpu_cores.iter().map(|core| {
                    let fraction = (core.percent / 100.0).clamp(0.0, 1.0);
                    div()
                        .flex_1()
                        .h(px((42.0 * fraction).max(2.0)))
                        .rounded_t_sm()
                        .bg(usage_color(core.percent, p))
                }),
            ));
        }
        section.into_any_element()
    }

    fn render_memory_section(&self, stats: &SystemStats, p: &Palette) -> AnyElement {
        let fraction = if stats.mem_total_bytes == 0 {
            0.0
        } else {
            stats.mem_used_bytes as f32 / stats.mem_total_bytes as f32
        };
        v_flex()
            .gap_2()
            .child(self.metric_header(
                "monitor.mem",
                format!(
                    "{} / {}",
                    format_size(stats.mem_used_bytes),
                    format_size(stats.mem_total_bytes)
                ),
                p.primary,
                p,
            ))
            .child(usage_bar(fraction, p.primary, p))
            .into_any_element()
    }

    fn render_filesystems_section(&self, stats: &SystemStats, p: &Palette) -> AnyElement {
        let mut section = v_flex()
            .gap_2()
            .child(self.section_title("monitor.disk", p));
        if stats.filesystems.is_empty() {
            section = section.child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("monitor.no_data").to_string()),
            );
        }
        for filesystem in stats.filesystems.iter().take(8) {
            let fraction = if filesystem.total_bytes == 0 {
                0.0
            } else {
                filesystem.used_bytes as f32 / filesystem.total_bytes as f32
            };
            let color = usage_color(fraction * 100.0, p);
            section = section.child(
                v_flex()
                    .gap_1()
                    .child(
                        h_flex()
                            .justify_between()
                            .text_xs()
                            .child(
                                div()
                                    .text_color(p.foreground)
                                    .child(filesystem.mount.clone()),
                            )
                            .child(div().text_color(p.muted_foreground).child(format!(
                                "{} / {}",
                                format_size(filesystem.used_bytes),
                                format_size(filesystem.total_bytes)
                            ))),
                    )
                    .child(usage_bar(fraction, color, p)),
            );
        }
        section.into_any_element()
    }

    fn render_network_section(&self, p: &Palette) -> AnyElement {
        let mut section = v_flex()
            .gap_2()
            .child(self.section_title("monitor.network", p));
        let latest = self
            .monitor
            .network_history
            .back()
            .cloned()
            .unwrap_or_default();
        if latest.is_empty() {
            section = section.child(
                div()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!("monitor.no_data").to_string()),
            );
        }
        for interface in &latest {
            let history: Vec<u64> = self
                .monitor
                .network_history
                .iter()
                .filter_map(|sample| {
                    sample
                        .iter()
                        .find(|entry| entry.name == interface.name)
                        .map(|entry| entry.rx_bytes_per_sec)
                })
                .collect();
            let peak = history.iter().copied().max().unwrap_or(1).max(1);
            section = section.child(
                v_flex()
                    .gap_1()
                    .child(
                        h_flex()
                            .justify_between()
                            .text_xs()
                            .child(div().text_color(p.foreground).child(interface.name.clone()))
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(div().text_color(p.muted_foreground).child(format!(
                                        "↓ {}/s",
                                        format_size(interface.rx_bytes_per_sec)
                                    )))
                                    .child(div().text_color(p.muted_foreground).child(format!(
                                        "↑ {}/s",
                                        format_size(interface.tx_bytes_per_sec)
                                    ))),
                            ),
                    )
                    .child(h_flex().h(px(22.0)).items_end().gap_0p5().children(
                        history.iter().map(|value| {
                            let fraction = (*value as f32 / peak as f32).clamp(0.0, 1.0);
                            div()
                                .w(px(3.0))
                                .h(px((22.0 * fraction).max(1.0)))
                                .bg(p.primary.opacity(0.65))
                        }),
                    )),
            );
        }
        section.into_any_element()
    }

    fn render_process_section(&mut self, cx: &mut Context<Self>, p: &Palette) -> AnyElement {
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
            })
            .collect();
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
        let mut section = v_flex()
            .gap_2()
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(self.section_title("monitor.processes", p))
                            .child(
                                div().text_xs().text_color(p.muted_foreground).child(
                                    rust_i18n::t!("monitor.process_count", count = processes.len())
                                        .to_string(),
                                ),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .child(
                                Button::new("sort-process-cpu")
                                    .small()
                                    .label(rust_i18n::t!("monitor.sort_cpu").to_string())
                                    .selected(sort == ProcessSort::Cpu)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.monitor.sort = ProcessSort::Cpu;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("sort-process-mem")
                                    .small()
                                    .label(rust_i18n::t!("monitor.sort_mem").to_string())
                                    .selected(sort == ProcessSort::Memory)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.monitor.sort = ProcessSort::Memory;
                                        cx.notify();
                                    })),
                            ),
                    ),
            )
            .child(Input::new(&self.monitor.filter).small())
            .child(
                h_flex()
                    .gap_2()
                    .px_1()
                    .text_xs()
                    .text_color(p.muted_foreground)
                    .child(div().w(px(54.0)).child("PID"))
                    .child(
                        div()
                            .w(px(64.0))
                            .child(rust_i18n::t!("monitor.user").to_string()),
                    )
                    .child(div().w(px(48.0)).child("CPU"))
                    .child(div().w(px(48.0)).child("MEM"))
                    .child(
                        div()
                            .flex_1()
                            .child(rust_i18n::t!("monitor.command").to_string()),
                    ),
            );
        for (index, process) in processes.iter().enumerate() {
            section = section.child(Self::render_process_row(index, process, &app_entity, p));
        }
        section.into_any_element()
    }

    /// 渲染一行进程：固定列宽 + 右键终止/复制菜单。
    fn render_process_row(
        index: usize,
        process: &ProcessInfo,
        app_entity: &Entity<AppShell>,
        p: &Palette,
    ) -> AnyElement {
        let pid = process.pid;
        let command = format!("{} {}", process.pid, process.cmdline);
        let app = app_entity.clone();
        let strong = app_entity.clone();
        let mut row = h_flex()
            .id(ElementId::Name(format!("process-row-{pid}").into()))
            .gap_2()
            .px_1()
            .h(px(22.0))
            .text_xs()
            .child(
                div()
                    .w(px(54.0))
                    .text_color(p.muted_foreground)
                    .child(pid.to_string()),
            )
            .child(
                div()
                    .w(px(64.0))
                    .truncate()
                    .text_color(p.foreground)
                    .child(process.user.clone()),
            )
            .child(
                div()
                    .w(px(48.0))
                    .text_color(usage_color(process.cpu_percent, p))
                    .child(format!("{:.1}", process.cpu_percent)),
            )
            .child(
                div()
                    .w(px(48.0))
                    .text_color(p.muted_foreground)
                    .child(format!("{:.1}", process.mem_percent)),
            )
            .child(
                div()
                    .flex_1()
                    .truncate()
                    .text_color(p.foreground)
                    .child(process.cmdline.clone()),
            )
            .context_menu(move |menu, _window, _cx| {
                let term = app.clone();
                let kill = strong.clone();
                let command_text = command.clone();
                menu.item(
                    PopupMenuItem::new(rust_i18n::t!("monitor.kill_term").to_string())
                        .icon(IconName::CircleX)
                        .on_click(move |_, _, cx| {
                            term.update(cx, |this, cx| this.send_kill_process(pid, false, cx));
                        }),
                )
                .item(
                    PopupMenuItem::new(rust_i18n::t!("monitor.kill_kill").to_string())
                        .icon(IconName::Ban)
                        .on_click(move |_, _, cx| {
                            kill.update(cx, |this, cx| this.send_kill_process(pid, true, cx));
                        }),
                )
                .item(
                    PopupMenuItem::new(rust_i18n::t!("monitor.copy_command").to_string())
                        .icon(IconName::Copy)
                        .on_click(move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(command_text.clone()));
                        }),
                )
            });
        if index % 2 == 1 {
            row = row.bg(p.muted.opacity(0.35));
        }
        if index % 2 == 1 {
            row = row.bg(p.muted.opacity(0.35));
        }
        row.into_any_element()
    }

    fn section_title(&self, key: &'static str, p: &Palette) -> AnyElement {
        div()
            .text_xs()
            .text_color(p.muted_foreground)
            .child(rust_i18n::t!(key).to_string())
            .into_any_element()
    }

    fn metric_header(
        &self,
        key: &'static str,
        value: String,
        color: Hsla,
        p: &Palette,
    ) -> AnyElement {
        h_flex()
            .justify_between()
            .text_xs()
            .child(
                div()
                    .text_color(p.muted_foreground)
                    .child(rust_i18n::t!(key).to_string()),
            )
            .child(div().text_color(color).child(value))
            .into_any_element()
    }
}
