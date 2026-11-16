//! 工作区纯布局计算：抽屉槽位判定与 SFTP 宽度计划。

use crate::sftp::SftpPanelState;
use crate::theme::tokens::{rem_px, size};
use gpui_kit::Pixels;

/// 尺寸表的最后一项是否就是抽屉槽位。
///
/// 设计原理（Why）：`ResizableState` 按面板索引记录尺寸，抽屉没打开时它没有
/// 槽位，最后一项属于 SFTP 或主面板。只有长度正好是“抽屉之前的面板数 + 1”
/// 时，最后一项才是抽屉自己的宽度；这同样是单帧跳动的根因，必须显式判断。
pub(super) fn drawer_slot_measured(sizes_len: usize, panels_before_drawer: usize) -> bool {
    sizes_len == panels_before_drawer + 1
}

/// SFTP 列的表面可见性、本地单列与折叠宽度计划。
pub(super) struct SftpLayoutPlan {
    pub(super) show: bool,
    pub(super) width: f32,
    pub(super) min_width: f32,
}

/// 计算 SFTP 面板宽度：全收起按内容宽、本地单列较窄、其余按默认。
pub(super) fn sftp_layout_plan(sftp_panel: Option<&SftpPanelState>, rem: Pixels) -> SftpLayoutPlan {
    let sftp_local_only = sftp_panel.is_some_and(|panel| panel.local.only);
    let sftp_all_collapsed = sftp_panel.is_some_and(|panel| panel.is_all_collapsed());
    let collapsed_count = sftp_panel
        .map(|panel| panel.collapsed_columns_count())
        .unwrap_or(0);
    // 全收起时的目标宽度必须包含面板自身边框，否则折叠状态下每次按内容
    // 宽度回写外层槽位都会少 1px，面板会逐帧向右收缩。
    let sftp_collapsed_width = sftp_panel
        .map(SftpPanelState::all_collapsed_width)
        .unwrap_or(32.0);
    let (sftp_width, mut sftp_min_width) = if sftp_all_collapsed {
        (sftp_collapsed_width, sftp_collapsed_width)
    } else if sftp_local_only && sftp_panel.is_some_and(|panel| panel.side_remotes.is_empty()) {
        (
            rem_px(size::SFTP_LOCAL_PANEL_WIDTH, rem).as_f32(),
            rem_px(size::SFTP_LOCAL_MIN_WIDTH, rem).as_f32(),
        )
    } else {
        (
            rem_px(size::SFTP_PANEL_WIDTH, rem).as_f32(),
            rem_px(size::SFTP_MIN_WIDTH, rem).as_f32(),
        )
    };
    // 有列折叠时允许 SFTP 面板收缩到最小内容宽度（折叠条 + 展开列保底宽度）：
    // 固定的 SFTP_MIN_WIDTH 会把面板卡在更宽的位置，导致折叠后无法收缩。
    if collapsed_count > 0
        && !sftp_all_collapsed
        && let Some(panel) = sftp_panel
    {
        sftp_min_width = sftp_min_width.min(panel.min_content_width());
    }
    SftpLayoutPlan {
        show: sftp_panel.is_some_and(|panel| panel.open),
        width: sftp_width,
        min_width: sftp_min_width,
    }
}

#[cfg(test)]
mod tests {
    use super::drawer_slot_measured;

    /// 尺寸表没补上抽屉槽位之前，末项属于别的面板，不能当抽屉宽度回写。
    ///
    /// 回归：抽屉打开的瞬间尺寸表仍是“侧栏 + 主面板 + SFTP”，末项是 SFTP 的
    /// 实测宽度，曾被当成抽屉宽度写回状态与设置，抽屉因此先按错误宽度出现
    /// 一帧再跳回来。
    #[test]
    fn drawer_width_is_not_taken_from_neighbour_panel() {
        // 三个面板、抽屉刚打开：尺寸表还没有抽屉槽位
        assert!(!drawer_slot_measured(3, 3));
        // 下一帧布局补上抽屉槽位
        assert!(drawer_slot_measured(4, 3));
        // 没有 SFTP 时同理：侧栏 + 主面板
        assert!(!drawer_slot_measured(2, 2));
        assert!(drawer_slot_measured(3, 2));
        // 抽屉关闭后尺寸表被截断，不能再回写
        assert!(!drawer_slot_measured(3, 3));
    }
}
