//! 进程管理板块（虚拟滚动列表、完整命令行详情条、终端追踪与防误杀保护）。

use crate::monitor::components::CollapsibleCard;
use crate::monitor::types::ProcessSort;
use crate::palette::Palette;
use gpui_kit::assets::IconName;
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::input::Input;
use gpui_kit::component::{Icon, Sizable};
use gpui_kit::*;

/// 进程列表可视区高度。
///
/// 固定高度是虚拟滚动生效的前提：父容器高度不确定时列表会按内容全量展开，
/// 几百个进程就会一次性全部渲染。
pub(super) const PROCESS_LIST_HEIGHT: f32 = 320.0;

/// 进程行高：虚拟列表按申报高度定位行，行内锁死同样高度才不会错位。
/// 20px 偏挤，取 24px 与组件库列表行的常规留白一致。
pub(super) const PROCESS_ROW_HEIGHT: f32 = 24.0;

/// 可排序表头列的静态描述，避免渲染函数参数膨胀。
pub(super) struct SortColumn {
    /// 宿主位置（抽屉 / 全屏标签页），用于生成唯一元素 id。
    pub(super) scope: &'static str,
    pub(super) label: String,
    pub(super) width: f32,
    pub(super) field: ProcessSort,
}

/// 进程表列宽：表头与数据行共用，保证两行列对齐。
pub(super) const COL_PID: f32 = 52.0;
pub(super) const COL_USER: f32 = 60.0;
pub(super) const COL_CPU: f32 = 44.0;
pub(super) const COL_MEM: f32 = 88.0;

use crate::monitor::MonitorView;

/// 进程内容渲染输入：排序结果、表头状态与作用域。
struct ProcessContent {
    order: Vec<usize>,
    sort: ProcessSort,
    ascending: bool,
    scope: &'static str,
}

impl MonitorView {
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
        let collapsed = self.state.collapse.processes;
        let total = self.state.process_total;
        let sort = self.state.sort;
        let ascending = self.state.sort_ascending;

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

        let content = self.render_process_content(
            ProcessContent {
                order,
                sort,
                ascending,
                scope,
            },
            cx,
            p,
        );

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
                let expanded = this.toggle_collapse("processes");
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

    /// 进程内容：过滤输入、表头、虚拟列表与选中行详情。
    fn render_process_content(
        &mut self,
        content_input: ProcessContent,
        cx: &mut Context<Self>,
        p: &Palette,
    ) -> AnyElement {
        let ProcessContent {
            order,
            sort,
            ascending,
            scope,
        } = content_input;
        let mut content = v_flex()
            .gap_2()
            .child(Input::new(&self.state.filter).small())
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

        content.into_any_element()
    }

    /// 过滤与排序后的行下标，按“筛选词 + 排序 + 快照版本”缓存。
    ///
    /// 表头计数与虚拟列表数据都走这里，保证条数与实际行数不会错位。进程表
    /// 每 5 秒才更新一轮，但渲染可能因光标闪烁/布局变化每帧发生：缓存让
    /// 逐进程小写比较与排序只在筛选变化或新样本到达时执行。
    fn filtered_order(&mut self, cx: &mut Context<Self>) -> Vec<usize> {
        let filter_value = self.state.filter.read(cx).value();
        let sort = self.state.sort;
        let ascending = self.state.sort_ascending;
        let revision = self.state.process_revision;
        if let Some((cached, cached_sort, cached_ascending, cached_revision, order)) =
            &self.state.process_order_cache
            && *cached == filter_value
            && *cached_sort == sort
            && *cached_ascending == ascending
            && *cached_revision == revision
        {
            return order.clone();
        }

        let filter = filter_value.to_lowercase();
        let processes = &self.state.processes;
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
        match sort {
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
        self.state.process_order_cache =
            Some((filter_value, sort, ascending, revision, order.clone()));
        order
    }

    /// 底部详情条：展示选中进程的完整命令行。
    fn render_process_detail(&mut self, p: &Palette) -> Option<AnyElement> {
        let pid = self.state.selected_process_pid?;
        let process = self
            .state
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
