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
use gpui_kit::component::resizable::resizable_panel;
use gpui_kit::*;
use transom_core::TabId;

mod view;
mod width;

pub(crate) use super::state::SftpWidthAdjustment;
pub(crate) use width::{column_layout, plan_side_mount};

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
pub(crate) struct SftpColumn {
    pub(super) id: SftpColumnId,
    pub(super) collapsed: bool,
    pub(super) content: AnyElement,
}
