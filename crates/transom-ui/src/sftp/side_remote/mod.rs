//! SFTP 并列远端的挂载/折叠状态与跨列剪贴板、拖放动作。
//!
//! 列状态与渲染已统一到 `SftpRemotePaneState` 与 `remote_pane`；本模块只保留
//! 挂载生命周期、连接查询与跨列传输编排。
//!
//! 设计原理（Why）：
//! - 实现初衷：支持在单个终端标签的 SFTP 视图中挂载多个远端会话（本地 + 远端 A + 远端 B...），
//!   免去在多个标签页之间反复来回切换，直观实现同屏比对与跨主机流式复制；
//! - 核心优势：
//!   1. 各列（本地列与并列远端列）均支持独立一键收起折叠，最大化屏幕有效阅读空间；
//!   2. 支持跨列拖拽（Drag & Drop）、Ctrl+C / Ctrl+V 剪贴板与右键菜单三重交互闭环；
//!   3. 挂载器支持复用当前已建立连接的 SSH 会话（秒级挂载）或快速连接已保存会话；
//! - 代价与局限：并列多列受屏幕实际宽度影响，单列过窄时由横向自适应和列折叠共同吸收。

mod types;

pub(crate) use self::types::{
    SftpActivePane, SftpClipboardData, SftpClipboardEntry, SftpClipboardSource, SftpDragItem,
    SftpDragPreview, SftpRemoteCopyParams, active_pane_after_removing,
};

use crate::sftp::SftpPanelState;
use gpui_kit::*;

/// 新挂载并列远端列的建议宽度。
///
/// 取当前展开列总宽按"挂载后列数"均分，再夹到可读区间：下限保证两行工具栏
/// 与路径框有基本空间，上限避免新列一上来就吃掉主远端；同时给现有列留出
/// 各自的最小宽度，面板本身很窄时不把已有列挤到裁切。分栏组还没有测量值时
/// 退回固定默认宽度。
pub(crate) fn side_column_width_hint(panel: &SftpPanelState, cx: &App) -> f32 {
    let sizes = panel.column_state.read(cx).sizes();
    let total: f32 = sizes.iter().map(|size| f32::from(*size)).sum();
    let columns = panel.expanded_column_ids().len().max(1);
    let fair = if total > 0.0 {
        total / (columns + 1) as f32
    } else {
        300.0
    };
    let hint = fair.clamp(300.0, 420.0);
    if total > 0.0 {
        // 现有每列至少保留 100px，剩下的才给新列；面板过窄时宁可让新列
        // 窄一些，也不把已有列压到裁切。
        hint.min((total - 100.0 * columns as f32).max(100.0))
    } else {
        hint
    }
}
