//! SFTP 面板的多列编排：列宽记忆、折叠条与可拖拽分栏。
//!
//! 设计原理（Why）：
//! - 所有列始终参与同一个 `h_resizable` 分组：折叠列是 32px 固定面板，展开列按
//!   记忆宽度布局，最后一个展开列吸收剩余宽度。这样任意列折叠后仍保留每条分隔
//!   条和其余相邻列的拖拽能力，也不会因为面板最小宽度在右侧留下空白；
//! - 宽度记忆按列身份保存，折叠列不覆盖其展开宽度，展开时恢复原宽度；
//! - 代价与局限：夹在两条折叠条之间的两个展开列没有直接相邻，拖拽需要经过折叠条，
//!   这是"折叠条仍占本列位置"的结果。

use super::state::SftpPanelState;
use crate::app::AppShell;
use crate::divider::h_resizable;
use crate::palette::Palette;
use gpui_kit::component::resizable::{ResizableState, resizable_panel};
use gpui_kit::*;
use transom_core::TabId;

/// 折叠列固定占用的宽度。
const COLLAPSED_COLUMN_WIDTH: f32 = 32.0;

/// 展开列的最小可读宽度。
const MIN_EXPANDED_COLUMN_WIDTH: f32 = 100.0;

/// SFTP 面板自身的横向边框：列区宽度比外层分栏槽位窄这么多。
///
/// 面板容器带 1px 左边框，列区实际可用宽度是 `槽位宽度 - 1`。折叠后按内容
/// 宽度反向收缩外层槽位时若忽略这 1px，每帧都会再要求缩小 1px，表现为面板
/// 一点点向右移动。所有“内容宽度 → 外层宽度”的换算都必须补上它。
const SFTP_PANEL_CHROME_WIDTH: f32 = 1.0;

/// 挂载新列后的宽度规划结果。
///
/// 设计原理（Why）：新挂载列是分栏组里唯一的柔性列，剩余宽度全部由它吸收；
/// 若现有列仍按记忆宽度占满面板，新列会被压成 0 像素，表现为整列空白。挂载
/// 时必须先把现有展开列重新分配，让出 `hint`；面板本身放不下时再申请加宽。
pub(super) struct SideMountPlan {
    /// 现有展开列的新宽度。
    pub(super) widths: Vec<(SftpColumnId, f32)>,
    /// 现有面板宽度不足以容纳新列时，需要额外增加的像素。
    pub(super) extra_width: f32,
}

/// 按实测列宽规划挂载后的现有列宽度。
///
/// 返回 `None` 表示分栏组还没测量过（例如本地单栏面板尚未进入多列布局），
/// 此时保持原有列宽、让新列直接吸收柔性空间即可。
pub(super) fn plan_side_mount(
    panel: &SftpPanelState,
    hint: f32,
    cx: &App,
) -> Option<SideMountPlan> {
    let layout = column_layout(panel);
    let sizes = panel.column_state.read(cx).sizes().clone();
    if sizes.len() != layout.len() {
        return None;
    }
    let collapsed_width =
        layout.iter().filter(|(_, collapsed)| *collapsed).count() as f32 * COLLAPSED_COLUMN_WIDTH;
    let total_width: f32 = sizes.iter().map(|width| f32::from(*width)).sum();
    let existing: Vec<(SftpColumnId, f32)> = layout
        .iter()
        .zip(sizes.iter())
        .filter(|((_, collapsed), _)| !*collapsed)
        .map(|((id, _), width)| (*id, f32::from(*width)))
        .collect();
    Some(plan_mount_widths(
        &existing,
        collapsed_width,
        hint,
        total_width,
    ))
}

/// 计算挂载后的现有列宽度：从可让出的余量里给新列挤出 `hint`。
///
/// 现有列各自保留 100px 下限，只按超出部分等比例让步；面板不足以同时满足
/// 下限与 `hint` 时，返回需要加宽的像素，由调用方扩展 SFTP 面板。
fn plan_mount_widths(
    existing: &[(SftpColumnId, f32)],
    collapsed_width: f32,
    hint: f32,
    total_width: f32,
) -> SideMountPlan {
    let count = existing.len();
    let min_total = MIN_EXPANDED_COLUMN_WIDTH * count as f32;
    // 现有列可占用的目标总量：总宽减去折叠条与新列后，不低于各自下限。
    let available = (total_width - collapsed_width - hint)
        .max(min_total)
        .max(0.0);
    let extra_total: f32 = existing
        .iter()
        .map(|(_, width)| (width - MIN_EXPANDED_COLUMN_WIDTH).max(0.0))
        .sum();
    let extra_budget = (available - min_total).max(0.0);
    let widths = existing
        .iter()
        .map(|(id, width)| {
            let extra = if extra_total > 0.0 {
                (width - MIN_EXPANDED_COLUMN_WIDTH).max(0.0) * extra_budget / extra_total
            } else if count > 0 {
                extra_budget / count as f32
            } else {
                0.0
            };
            (*id, MIN_EXPANDED_COLUMN_WIDTH + extra)
        })
        .collect::<Vec<_>>();
    let planned_total =
        collapsed_width + widths.iter().map(|(_, width)| *width).sum::<f32>() + hint;
    SideMountPlan {
        widths,
        extra_width: (planned_total - total_width).max(0.0),
    }
}

/// 折叠态下面板内容所需宽度：折叠条按条宽，展开列按记忆宽度。
fn sftp_columns_content_width(panel: &SftpPanelState) -> Option<f32> {
    let mut expanded = Vec::with_capacity(panel.expanded_column_ids().len());
    for id in panel.expanded_column_ids() {
        expanded.push(panel.column_widths.get(id)?);
    }
    Some(collapsed_content_width(
        panel.collapsed_columns_count(),
        &expanded,
    ))
}

/// 折叠态外层槽位宽度：面板边框 + 折叠条 + 展开列宽度之和。
///
/// 这是“内容宽度 → 外层槽位宽度”的唯一换算口；漏掉边框会让折叠后的宽度
/// 反馈循环每帧少 1px。
fn collapsed_content_width(collapsed_count: usize, expanded_widths: &[f32]) -> f32 {
    SFTP_PANEL_CHROME_WIDTH
        + collapsed_count as f32 * COLLAPSED_COLUMN_WIDTH
        + expanded_widths.iter().sum::<f32>()
}

/// 按渲染顺序返回所有列的身份与折叠状态。
///
/// 分栏状态始终包含全部列（折叠列是 32px 固定面板），所以宽度测量与列身份
/// 必须按完整顺序对齐，不能只取展开列。
fn column_layout(panel: &SftpPanelState) -> Vec<(SftpColumnId, bool)> {
    let mut layout = Vec::with_capacity(2 + panel.side_remotes.len());
    layout.push((SftpColumnId::Local, panel.local.collapsed));
    if !panel.local.only {
        layout.push((SftpColumnId::Remote, panel.remote.collapsed));
    }
    for (index, side) in panel.side_remotes.iter().enumerate() {
        layout.push((SftpColumnId::Side(index), side.collapsed));
    }
    layout
}

impl SftpPanelState {
    /// 折叠态内容宽度：折叠条 + 展开列的记忆宽度；有列没有记忆宽度时返回 `None`。
    pub(crate) fn collapsed_content_width(&self) -> Option<f32> {
        sftp_columns_content_width(self)
    }

    /// 全收起时的外层槽位宽度：折叠条总宽 + 面板自身边框。
    pub(crate) fn all_collapsed_width(&self) -> f32 {
        SFTP_PANEL_CHROME_WIDTH + self.collapsed_columns_count() as f32 * COLLAPSED_COLUMN_WIDTH
    }
}

/// SFTP 面板内的列身份。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SftpColumnId {
    Local,
    Remote,
    /// 并列远端列，按挂载顺序编号。
    Side(usize),
}

/// 各列宽度记忆：按列身份存放，折叠导致列数变化时不会错位到别的列。
#[derive(Debug, Default, Clone)]
pub(crate) struct ColumnWidthMemory {
    widths: Vec<(SftpColumnId, f32)>,
}

impl ColumnWidthMemory {
    /// 记住某列的实际宽度（像素）。
    pub(crate) fn remember(&mut self, id: SftpColumnId, width: f32) {
        match self.widths.iter_mut().find(|(known, _)| *known == id) {
            Some(entry) => entry.1 = width,
            None => self.widths.push((id, width)),
        }
    }

    /// 取某列记住的宽度；没有记忆时返回 `None`，由调用方决定退化行为。
    pub(crate) fn get(&self, id: SftpColumnId) -> Option<f32> {
        self.widths
            .iter()
            .find(|(known, _)| *known == id)
            .map(|(_, width)| *width)
    }
}

/// 面板中的一列：身份、折叠状态与将要渲染的内容（展开列或折叠条）。
pub(super) struct SftpColumn {
    pub(super) id: SftpColumnId,
    pub(super) collapsed: bool,
    pub(super) content: AnyElement,
}

impl AppShell {
    /// 折叠或展开后同步 SFTP 面板的整体宽度。
    ///
    /// 折叠时面板收到内容宽度（折叠条 + 展开列的记忆宽度），让出的空间归终端；
    /// 展开时恢复折叠前记下的宽度。展开列的宽度记忆始终不变。
    ///
    /// 放在渲染路径而不是折叠动作里：`resize_panel` 需要可变窗口句柄，
    /// 而右键菜单的回调只能拿到不可变窗口。
    pub(crate) fn sync_sftp_panel_width(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab_id) = self.workspace.active_tab else {
            return;
        };
        let Some(panel) = self.workspace.tab(tab_id).and_then(|tab| tab.sftp()) else {
            self.sftp_panel_width_target = None;
            return;
        };
        let collapsed = panel.collapsed_columns_count() > 0;
        let Some(index) = self.sftp_resizable_index() else {
            return;
        };
        let panels = if self.settings.window.sftp_panel_side.is_left() {
            self.workspace_panels_left.clone()
        } else {
            self.workspace_panels.clone()
        };
        // 目标宽度：折叠时是内容宽度，展开时是折叠前记下的宽度。
        let desired = {
            let Some(panel) = self.sftp_panel_mut_tab(tab_id) else {
                return;
            };
            if collapsed {
                sftp_columns_content_width(panel)
            } else {
                panel.restored_width
            }
        };
        if collapsed {
            // 折叠前的宽度只记一次：连续折叠多列时不覆盖最初那个值。
            let Some(panel) = self.sftp_panel_mut_tab(tab_id) else {
                return;
            };
            if panel.restored_width.is_none() {
                panel.restored_width = panels
                    .read(cx)
                    .sizes()
                    .get(index)
                    .copied()
                    .map(|width| width.as_f32());
            }
        } else if let Some(panel) = self.sftp_panel_mut_tab(tab_id) {
            // 恢复目标已经捕获到 desired 里，这里清掉记录，后续按当前宽度重新开始。
            panel.restored_width = None;
        }
        // 以分栏状态里的实际宽度判断是否需要调整：目标宽度可能与上一帧的
        // size_range 最小值不一致而被夹住，只看记录的目标值会永久停在错误宽度。
        let actual = panels
            .read(cx)
            .sizes()
            .get(index)
            .copied()
            .map(|width| width.as_f32());
        if let Some(target) = desired
            && actual.is_some_and(|width| (width - target).abs() < 0.5)
        {
            self.sftp_panel_width_target = desired;
            return;
        }
        if desired.is_none() && self.sftp_panel_width_target.is_none() {
            return;
        }
        self.sftp_panel_width_target = desired;
        let Some(target) = desired else {
            return;
        };
        panels.update(cx, |state, cx| {
            state.resize_panel(index, px(target), window, cx)
        });
    }

    /// 把 SFTP 面板加宽 `delta`：挂载新列时现有面板放不下，直接向工作区分栏申请空间。
    pub(super) fn grow_sftp_panel_width(
        &mut self,
        delta: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if delta <= 0.5 {
            return;
        }
        let Some(index) = self.sftp_resizable_index() else {
            return;
        };
        let panels = if self.settings.window.sftp_panel_side.is_left() {
            self.workspace_panels_left.clone()
        } else {
            self.workspace_panels.clone()
        };
        let Some(current) = panels
            .read(cx)
            .sizes()
            .get(index)
            .copied()
            .map(|width| width.as_f32())
        else {
            return;
        };
        panels.update(cx, |state, cx| {
            state.resize_panel(index, px(current + delta), window, cx)
        });
    }

    /// SFTP 面板在工作区分组里的面板索引。
    ///
    /// 侧栏恒占索引 0（不可见时也保留占位以稳定索引），SFTP 停靠左侧时紧随其后，
    /// 停靠右侧时排在主面板之后。
    fn sftp_resizable_index(&self) -> Option<usize> {
        if self.immersive {
            return None;
        }
        Some(if self.settings.window.sftp_panel_side.is_left() {
            1
        } else {
            2
        })
    }

    /// 折叠前把当前分栏尺寸记进列宽记忆。
    ///
    /// 必须在折叠发生的那一刻取：之后列宽会按折叠状态重排。折叠列本身不覆盖
    /// 宽度记忆，展开时要恢复它原来的展开宽度。
    pub(super) fn remember_sftp_column_widths(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(panel) = self.sftp_panel_mut_tab(tab_id) else {
            return;
        };
        let layout = column_layout(panel);
        let sizes = panel.column_state.read(cx).sizes().clone();
        // 状态刚重建时尺寸数量可能与新布局不一致，等下一页测量完成再记。
        if sizes.len() != layout.len() {
            return;
        }
        for ((id, collapsed), width) in layout.iter().zip(sizes.iter()) {
            if !*collapsed {
                panel.column_widths.remember(*id, f32::from(*width));
            }
        }
    }

    /// 折叠状态变化后重建分栏状态：下一次渲染用列宽记忆与固定折叠条宽度重新初始化。
    ///
    /// 直接修改旧状态的尺寸会与“折叠列 32px、展开列记忆宽度”的初始布局冲突；
    /// 重建状态只在折叠/展开/挂载/关闭的瞬间发生，拖拽期间状态保持不动。
    pub(crate) fn reset_sftp_column_state(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if let Some(panel) = self.sftp_panel_mut_tab(tab_id) {
            panel.column_state = cx.new(|_| ResizableState::default());
        }
    }

    /// 渲染 SFTP 面板的列区：所有列始终在同一个可拖拽分栏组里。
    pub(super) fn render_sftp_columns(
        &mut self,
        tab_id: TabId,
        columns: Vec<SftpColumn>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if columns.is_empty() {
            return div().into_any_element();
        }
        self.render_sftp_resizable_columns(tab_id, columns, cx)
    }

    /// 构建分栏组：折叠条是固定 32px 面板，展开列按记忆宽度，最后一个展开列吸收余量。
    fn render_sftp_resizable_columns(
        &mut self,
        tab_id: TabId,
        columns: Vec<SftpColumn>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let column_state = self
            .sftp_panel_mut_tab(tab_id)
            .map(|panel| panel.column_state.clone());
        let width_memory = self
            .sftp_panel_mut_tab(tab_id)
            .map(|panel| panel.column_widths.clone());
        let last_expanded = columns
            .iter()
            .rev()
            .find(|column| !column.collapsed)
            .map(|column| column.id);

        let mut resizable =
            h_resizable(ElementId::Name(format!("sftp-multi-pane-{tab_id}").into()));
        if let Some(state) = &column_state {
            // 绑定面板自己持有的状态：窗口的 keyed state 会在元素不再渲染时被回收，
            // 折叠期间分栏组不渲染，尺寸必须另有去处。
            resizable = resizable.with_state(state);
        }
        let mut identities = Vec::with_capacity(columns.len());
        for column in columns {
            identities.push((column.id, column.collapsed));
            let panel = if column.collapsed {
                // 折叠条固定 32px：size_range 锁死，拖自己的手柄也不会把条拉宽；
                // 展开列之间的分隔仍由相邻面板的手柄负责。
                resizable_panel()
                    .size(px(COLLAPSED_COLUMN_WIDTH))
                    .size_range(px(COLLAPSED_COLUMN_WIDTH)..px(COLLAPSED_COLUMN_WIDTH))
                    .fixed()
            } else if Some(column.id) == last_expanded {
                // 最后一个展开列吸收剩余宽度：面板被最小宽度限制或窗口变宽时，
                // 余量落在它身上，不会在分栏组右侧留白。
                resizable_panel()
            } else if let Some(width) = width_memory
                .as_ref()
                .and_then(|memory| memory.get(column.id))
            {
                resizable_panel().size(px(width))
            } else {
                resizable_panel()
            };
            resizable = resizable.child(panel.child(column.content));
        }
        // 宽度记忆：每帧从分栏状态读回展开列实际宽度；折叠条不覆盖展开宽度。
        if let Some(state) = &column_state {
            let sizes = state.read(cx).sizes().clone();
            if let Some(panel) = self.sftp_panel_mut_tab(tab_id) {
                for ((id, collapsed), width) in identities.iter().zip(sizes.iter()) {
                    if !*collapsed {
                        panel.column_widths.remember(*id, f32::from(*width));
                    }
                }
            }
        }
        resizable.into_any_element()
    }

    /// 按列身份渲染对应的折叠条。
    pub(super) fn render_sftp_collapsed_bar(
        &self,
        tab_id: TabId,
        id: SftpColumnId,
        side_titles: &[(usize, String)],
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match id {
            SftpColumnId::Local => self.render_sftp_local_collapsed_bar(tab_id, p, cx),
            SftpColumnId::Remote => self.render_sftp_remote_collapsed_bar(tab_id, p, cx),
            SftpColumnId::Side(index) => {
                let title = side_titles
                    .iter()
                    .find(|(known, _)| *known == index)
                    .map(|(_, title)| title.clone())
                    .unwrap_or_default();
                self.render_sftp_side_remote_collapsed_bar(tab_id, index, &title, p, cx)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        COLLAPSED_COLUMN_WIDTH, ColumnWidthMemory, MIN_EXPANDED_COLUMN_WIDTH,
        SFTP_PANEL_CHROME_WIDTH, SftpColumnId, collapsed_content_width, plan_mount_widths,
    };

    /// 收起后的目标宽度必须包含面板边框，否则外层槽位每帧再缩 1px。
    #[test]
    fn collapsed_width_includes_panel_chrome() {
        let expanded_width = 250.0;
        let outer = collapsed_content_width(1, &[expanded_width]);
        assert!(
            (outer - (SFTP_PANEL_CHROME_WIDTH + COLLAPSED_COLUMN_WIDTH + expanded_width)).abs()
                < 0.01
        );
        // 外层槽位扣除边框后正好容纳折叠条与展开列，下一帧目标不再变化。
        let inner = outer - SFTP_PANEL_CHROME_WIDTH;
        assert!((inner - (COLLAPSED_COLUMN_WIDTH + expanded_width)).abs() < 0.01);
    }

    /// 宽度记忆按列身份存放：折叠导致列数变化时，宽度不会记到别的列头上。
    #[test]
    fn widths_are_remembered_by_identity() {
        let mut memory = ColumnWidthMemory::default();
        memory.remember(SftpColumnId::Local, 300.0);
        memory.remember(SftpColumnId::Remote, 280.0);
        memory.remember(SftpColumnId::Side(0), 120.0);

        assert_eq!(memory.get(SftpColumnId::Local), Some(300.0));
        assert_eq!(memory.get(SftpColumnId::Remote), Some(280.0));
        assert_eq!(memory.get(SftpColumnId::Side(0)), Some(120.0));

        // 同一列再记一次是覆盖而不是追加。
        memory.remember(SftpColumnId::Remote, 260.0);
        assert_eq!(memory.get(SftpColumnId::Remote), Some(260.0));
        assert_eq!(
            memory.get(SftpColumnId::Local),
            Some(300.0),
            "其它列的记忆不受影响"
        );

        // 没有记忆的列返回 None，由调用方决定退化行为。
        assert_eq!(memory.get(SftpColumnId::Side(1)), None);
    }

    /// 面板宽裕时按现有余量等比例让步，新列拿到完整的建议宽度。
    #[test]
    fn mount_plan_keeps_hint_and_shrinks_existing_proportionally() {
        let existing = [(SftpColumnId::Local, 400.0), (SftpColumnId::Remote, 200.0)];
        let plan = plan_mount_widths(&existing, 0.0, 300.0, 600.0);

        assert_eq!(plan.extra_width, 0.0, "现有面板足够容纳新列");
        let sum: f32 = plan.widths.iter().map(|(_, width)| *width).sum();
        assert!((sum - 300.0).abs() < 0.01, "现有列让出 300px，剩余 300px");
        assert_eq!(plan.widths.len(), 2);
        assert!(plan.widths.iter().all(|(_, width)| *width >= 100.0));
        // 400:200 的余量按 300:100 分配，加回下限后是 175:125。
        let local = plan.widths[0].1;
        let remote = plan.widths[1].1;
        assert!((local - 175.0).abs() < 0.01 && (remote - 125.0).abs() < 0.01);
    }

    /// 折叠条占用的宽度不参与现有列分配，但计入面板总宽。
    #[test]
    fn mount_plan_accounts_for_collapsed_bars() {
        let existing = [(SftpColumnId::Remote, 600.0)];
        let collapsed_width = COLLAPSED_COLUMN_WIDTH;
        let plan = plan_mount_widths(&existing, collapsed_width, 300.0, 632.0);

        assert_eq!(plan.extra_width, 0.0);
        assert!((plan.widths[0].1 - 300.0).abs() < 0.01, "主远端让出空间");
        let occupied = collapsed_width + plan.widths[0].1 + 300.0;
        assert!((occupied - 632.0).abs() < 0.01, "总宽保持不变");
    }

    /// 面板太窄时现有列保底 100px，并返回需要加宽面板的像素。
    #[test]
    fn mount_plan_requests_extra_width_when_panel_is_too_narrow() {
        let existing = [(SftpColumnId::Local, 100.0)];
        let plan = plan_mount_widths(&existing, 0.0, 300.0, 100.0);

        assert!((plan.widths[0].1 - MIN_EXPANDED_COLUMN_WIDTH).abs() < 0.01);
        assert!(
            (plan.extra_width - 300.0).abs() < 0.01,
            "需要补足新列的建议宽度"
        );
    }

    /// 没有展开列时（仅折叠条）直接为新列申请宽度，不产生虚假的现有列。
    #[test]
    fn mount_plan_without_expanded_columns_requests_hint() {
        let plan = plan_mount_widths(&[], COLLAPSED_COLUMN_WIDTH, 300.0, 32.0);

        assert!(plan.widths.is_empty());
        assert!((plan.extra_width - 300.0).abs() < 0.01);
    }
}
