//! SFTP 多列分栏的 UI 测量、折叠与拖拽状态。

use super::width::COLLAPSED_COLUMN_WIDTH;
use super::*;
use crate::sftp::entity::SftpPanelView;

impl SftpPanelView {
    /// 渲染 SFTP 面板的列区：所有列始终在同一个可拖拽分栏组里。
    pub(crate) fn render_sftp_columns(
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
        let column_state = self.state.column_state.clone();
        let width_memory = self.state.column_widths.clone();
        let last_expanded = columns
            .iter()
            .rev()
            .find(|column| !column.collapsed)
            .map(|column| column.id);

        let mut resizable =
            h_resizable(ElementId::Name(format!("sftp-multi-pane-{tab_id}").into()));
        {
            resizable = resizable.with_state(&column_state);
        }
        let mut identities = Vec::with_capacity(columns.len());
        for column in columns {
            identities.push((column.id, column.collapsed));
            let panel = if column.collapsed {
                resizable_panel()
                    .size(px(COLLAPSED_COLUMN_WIDTH))
                    .size_range(px(COLLAPSED_COLUMN_WIDTH)..px(COLLAPSED_COLUMN_WIDTH))
                    .fixed()
            } else if Some(column.id) == last_expanded {
                resizable_panel()
            } else if let Some(width) = width_memory.get(column.id) {
                resizable_panel().size(px(width))
            } else {
                resizable_panel()
            };
            resizable = resizable.child(panel.child(column.content));
        }
        // 宽度记忆：每帧从分栏状态读回展开列实际宽度；折叠条不覆盖展开宽度。
        let sizes = column_state.read(cx).sizes().to_vec();
        for ((id, collapsed), width) in identities.iter().zip(sizes.iter()) {
            if !*collapsed {
                self.state.column_widths.remember(*id, f32::from(*width));
            }
        }
        resizable.into_any_element()
    }

    /// 按列身份渲染对应的折叠条。
    pub(crate) fn render_sftp_collapsed_bar(
        &self,
        id: SftpColumnId,
        side_titles: &[(usize, String)],
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match id {
            SftpColumnId::Local => self.render_sftp_local_collapsed_bar(p, cx),
            SftpColumnId::Remote => self.render_sftp_remote_collapsed_bar(p, cx),
            SftpColumnId::Side(index) => {
                let title = side_titles
                    .iter()
                    .find(|(known, _)| *known == index)
                    .map(|(_, title)| title.clone())
                    .unwrap_or_default();
                self.render_sftp_side_remote_collapsed_bar(index, &title, p, cx)
            }
        }
    }
}
