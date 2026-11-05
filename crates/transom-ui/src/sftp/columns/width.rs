//! 列宽规划与折叠宽度换算。

use super::*;

/// 折叠列固定占用的宽度。
pub(super) const COLLAPSED_COLUMN_WIDTH: f32 = 32.0;

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
pub(crate) struct SideMountPlan {
    /// 现有展开列的新宽度。
    pub(crate) widths: Vec<(SftpColumnId, f32)>,
    /// 现有面板宽度不足以容纳新列时，需要额外增加的像素。
    pub(crate) extra_width: f32,
}

/// 按实测列宽规划挂载后的现有列宽度。
///
/// 返回 `None` 表示分栏组还没测量过（例如本地单栏面板尚未进入多列布局），
/// 此时保持原有列宽、让新列直接吸收柔性空间即可。
pub(crate) fn plan_side_mount(
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

#[cfg(test)]
fn collapsed_content_width(collapsed_count: usize, expanded_widths: &[f32]) -> f32 {
    SFTP_PANEL_CHROME_WIDTH
        + collapsed_count as f32 * COLLAPSED_COLUMN_WIDTH
        + expanded_widths.iter().sum::<f32>()
}

/// 按渲染顺序返回所有列的身份与折叠状态。
///
/// 分栏状态始终包含全部列（折叠列是 32px 固定面板），所以宽度测量与列身份
/// 必须按完整顺序对齐，不能只取展开列。
pub(crate) fn column_layout(panel: &SftpPanelState) -> Vec<(SftpColumnId, bool)> {
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
    /// 面板内容所允许的最小物理宽度：
    /// 面板边框 + 折叠条（每个 32px） + 展开列下限（每个 100px）。
    pub(crate) fn min_content_width(&self) -> f32 {
        let collapsed_count = self.collapsed_columns_count();
        let expanded_count = self.expanded_column_ids().len();
        SFTP_PANEL_CHROME_WIDTH
            + collapsed_count as f32 * COLLAPSED_COLUMN_WIDTH
            + expanded_count as f32 * MIN_EXPANDED_COLUMN_WIDTH
    }

    /// 计算切换某列折叠状态后对面板整体宽度的调整需求。
    ///
    /// 设计原理（Why）：
    /// - 收起列时，面板向内收缩该列让出的宽度差额（`列宽 - 32px`），将多余空间归还主工作区；
    /// - 展开列时，面板在当前实测宽度基础上向外加宽（`列宽 - 32px`），保留用户在折叠期间对展开列的拉伸调整；
    /// - 全收起时直接使用固定折叠条总宽。
    pub(crate) fn plan_collapse_width_adjustment(
        &self,
        id: SftpColumnId,
        now_collapsed: bool,
    ) -> SftpWidthAdjustment {
        if self.is_all_collapsed() {
            return SftpWidthAdjustment::Exact(self.all_collapsed_width());
        }
        let col_width = self.column_widths.get(id).unwrap_or(280.0);
        let delta = (col_width - COLLAPSED_COLUMN_WIDTH).max(0.0);
        if now_collapsed {
            SftpWidthAdjustment::Delta(-delta)
        } else {
            SftpWidthAdjustment::Delta(delta)
        }
    }

    /// 全收起时的外层槽位宽度：折叠条总宽 + 面板自身边框。
    pub(crate) fn all_collapsed_width(&self) -> f32 {
        SFTP_PANEL_CHROME_WIDTH + self.collapsed_columns_count() as f32 * COLLAPSED_COLUMN_WIDTH
    }
}

#[cfg(test)]
mod tests {
    use super::{
        COLLAPSED_COLUMN_WIDTH, ColumnWidthMemory, MIN_EXPANDED_COLUMN_WIDTH,
        SFTP_PANEL_CHROME_WIDTH, SftpColumnId, SftpWidthAdjustment, collapsed_content_width,
        plan_mount_widths,
    };

    /// 收起本地时向内收缩差额，展开时向外扩展差额。
    #[test]
    fn plan_collapse_width_adjustment_delta_calculation() {
        let mut memory = ColumnWidthMemory::default();
        memory.remember(SftpColumnId::Local, 300.0);
        let col_width = memory.get(SftpColumnId::Local).unwrap();
        let delta = (col_width - COLLAPSED_COLUMN_WIDTH).max(0.0);
        assert!((delta - 268.0).abs() < 0.01);

        let collapse_adj = SftpWidthAdjustment::Delta(-delta);
        assert_eq!(collapse_adj, SftpWidthAdjustment::Delta(-268.0));

        let expand_adj = SftpWidthAdjustment::Delta(delta);
        assert_eq!(expand_adj, SftpWidthAdjustment::Delta(268.0));
    }

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
