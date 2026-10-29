//! SFTP 面板的多列编排：展开列的宽度记忆、折叠条位置与空白分配。
//!
//! 设计原理（Why）：
//! - 实现初衷：折叠一列不应该改变其它列的尺寸——用户给远端列拖过宽度，
//!   折叠本地列时那点宽度不该被吃掉。所以每列宽度按"列身份"记住，折叠时展开列
//!   固定用它，折叠条就贴在它旁边；展开列一律靠右对齐，多出来的宽度留白在左侧，
//!   收起远端列时本地列因此向右贴住折叠条，宽度与位置都不跳；
//! - 核心优势：宽度记忆按身份而非渲染下标存放，折叠导致列数变化时不会错位到别的列；
//! - 代价与局限：面板整体变宽时折叠中的展开列不跟随，多出的宽度全部留白——
//!   这是"折叠不改其它列尺寸"的直接后果，也是刻意选择。

use super::state::SftpPanelState;
use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::base::h_flex;
use gpui_kit::component::resizable::{h_resizable, resizable_panel};
use gpui_kit::*;
use transom_core::TabId;

/// 折叠态下面板内容所需宽度：折叠条按条宽，展开列按记忆宽度。
///
/// 展开列若还没有记忆宽度，说明它会占满剩余空间，内容宽度无从得知，返回 `None`，
/// 由调用方保持面板现状。
fn sftp_columns_content_width(panel: &SftpPanelState) -> Option<f32> {
    let mut width = panel.collapsed_columns_count() as f32 * 32.0;
    for id in panel.expanded_column_ids() {
        width += panel.column_widths.get(id)?;
    }
    Some(width)
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

/// 面板中的一列：身份与展开态内容（`None` 表示该列已折叠）。
pub(super) struct SftpColumn {
    pub(super) id: SftpColumnId,
    pub(super) content: Option<AnyElement>,
}

impl AppShell {
    /// 折叠或展开后同步 SFTP 面板的整体宽度。
    ///
    /// 折叠时面板收到内容宽度（折叠条 + 展开列的记忆宽度），让出的空间归终端；
    /// 展开时恢复折叠前记下的宽度。远端列自身的宽度始终不变。
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
        // 内容宽度随折叠的列数变化（一条 32px，展开列按记忆宽度），
        // 所以按目标值判断是否需要调整，而不是只看"是否处于折叠态"。
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
        if self.sftp_panel_width_target == desired {
            return;
        }
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
            panel.restored_width = None;
        }
        self.sftp_panel_width_target = desired;
        let Some(target) = desired else {
            return;
        };
        panels.update(cx, |state, cx| {
            state.resize_panel(index, px(target), window, cx)
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
    /// 必须在折叠发生的那一刻取：折叠后分栏组不再渲染，尺寸无处可读，
    /// 而展开列要按记忆固定宽度，远端才不会被拉宽。
    pub(super) fn remember_sftp_column_widths(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(panel) = self.sftp_panel_mut_tab(tab_id) else {
            return;
        };
        let identities = panel.expanded_column_ids();
        let sizes = panel.column_state.read(cx).sizes().clone();
        for (id, width) in identities.iter().zip(sizes.iter()) {
            panel.column_widths.remember(*id, f32::from(*width));
        }
    }

    /// 渲染 SFTP 面板的列区。
    ///
    /// 全部展开时用可拖动的分栏组，列宽由用户分配并被记住；一旦有列折叠，
    /// 展开列按记忆宽度固定并靠右对齐，折叠条占住自己那一列的位置，
    /// 多出来的宽度留白在左侧。
    pub(super) fn render_sftp_columns(
        &mut self,
        tab_id: TabId,
        columns: Vec<SftpColumn>,
        side_titles: &[(usize, String)],
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let all_expanded = columns.iter().all(|column| column.content.is_some());
        if all_expanded && columns.len() > 1 {
            return self.render_sftp_resizable_columns(tab_id, columns, cx);
        }
        self.render_sftp_fixed_columns(tab_id, columns, side_titles, p, cx)
    }

    /// 全展开时的分栏组：可拖动分隔条，并在此把各列实际宽度记进面板状态。
    fn render_sftp_resizable_columns(
        &mut self,
        tab_id: TabId,
        columns: Vec<SftpColumn>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let column_state = self
            .sftp_panel_mut_tab(tab_id)
            .map(|panel| panel.column_state.clone());
        let mut resizable =
            h_resizable(ElementId::Name(format!("sftp-multi-pane-{tab_id}").into()));
        if let Some(state) = &column_state {
            // 绑定面板自己持有的状态：窗口的 keyed state 会在元素不再渲染时被回收，
            // 折叠期间分栏组不渲染，尺寸必须另有去处。
            resizable = resizable.with_state(state);
        }
        let mut identities = Vec::with_capacity(columns.len());
        for column in columns {
            let Some(content) = column.content else {
                continue;
            };
            identities.push(column.id);
            resizable = resizable.child(resizable_panel().child(content));
        }
        // 宽度记忆：每帧从分栏状态读回各列实际宽度，按列身份存。
        if let Some(state) = &column_state {
            let sizes = state.read(cx).sizes().clone();
            if let Some(panel) = self.sftp_panel_mut_tab(tab_id) {
                for (id, width) in identities.iter().zip(sizes.iter()) {
                    panel.column_widths.remember(*id, f32::from(*width));
                }
            }
        }
        resizable.into_any_element()
    }

    /// 有列折叠时的排布：折叠条占自己那一列的位置，展开列按记忆宽度固定并靠右对齐。
    fn render_sftp_fixed_columns(
        &mut self,
        tab_id: TabId,
        columns: Vec<SftpColumn>,
        side_titles: &[(usize, String)],
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let widths = self
            .sftp_panel_mut_tab(tab_id)
            .map(|panel| panel.column_widths.clone());
        let mut container = h_flex().size_full().overflow_hidden().justify_end();
        for column in columns {
            match column.content {
                Some(content) => {
                    let width = widths.as_ref().and_then(|memory| memory.get(column.id));
                    let element = div().h_full().overflow_hidden().child(content);
                    container = container.child(match width {
                        Some(width) => element.w(px(width)),
                        // 还没有记忆宽度时退化为占满剩余空间，与旧行为一致。
                        None => element.flex_1(),
                    });
                }
                None => {
                    container = container.child(self.render_sftp_collapsed_bar(
                        tab_id,
                        column.id,
                        side_titles,
                        p,
                        cx,
                    ));
                }
            }
        }
        container.into_any_element()
    }

    /// 按列身份渲染对应的折叠条。
    fn render_sftp_collapsed_bar(
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
    use super::{ColumnWidthMemory, SftpColumnId};

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
}
