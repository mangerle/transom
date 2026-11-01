//! SFTP 多列分栏的 UI 测量、折叠与拖拽状态。

use super::width::{COLLAPSED_COLUMN_WIDTH, sftp_columns_content_width};
use super::*;
use crate::sftp::entity::SftpPanelView;
use transom_core::AppSettings;

impl AppShell {
    /// 折叠或展开后同步 SFTP 面板的整体宽度。
    ///
    /// 折叠时面板收到内容宽度（折叠条 + 展开列的记忆宽度），让出的空间归终端；
    /// 展开时恢复折叠前记下的宽度。展开列的宽度记忆始终不变。
    ///
    /// 放在渲染路径而不是折叠动作里：`resize_panel` 需要可变窗口句柄，
    /// 而右键菜单的回调只能拿到不可变窗口。
    pub(crate) fn sync_sftp_panel_width(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab_id) = self.state.workspace.active_tab else {
            return;
        };
        let Some((collapsed, desired)) = self.sftp_state(tab_id, cx).map(|panel| {
            (
                panel.collapsed_columns_count() > 0,
                if panel.collapsed_columns_count() > 0 {
                    sftp_columns_content_width(panel)
                } else {
                    panel.restored_width
                },
            )
        }) else {
            self.state.sftp_panel_width_target = None;
            return;
        };
        let Some(index) = self.sftp_resizable_index(&self.state.settings) else {
            return;
        };
        let panels = if self.state.settings.window.sftp_panel_side.is_left() {
            self.state.workspace_panels_left.clone()
        } else {
            self.state.workspace_panels.clone()
        };
        let current_width = panels
            .read(cx)
            .sizes()
            .get(index)
            .copied()
            .map(|width| width.as_f32());
        if collapsed
            && let Some(mut panel) = self.sftp_view_mut(tab_id, cx)
            && panel.restored_width.is_none()
        {
            // 折叠前的宽度只记一次：连续折叠多列时不覆盖最初那个值。
            panel.restored_width = current_width;
        } else if let Some(mut panel) = self.sftp_view_mut(tab_id, cx) {
            // 恢复目标已经捕获到 desired 里，这里清掉记录，后续按当前宽度重新开始。
            panel.restored_width = None;
        }
        // 以分栏状态里的实际宽度判断是否需要调整：目标宽度可能与上一帧的
        // size_range 最小值不一致而被夹住，只看记录的目标值会永久停在错误宽度。
        let actual = current_width;
        if let Some(target) = desired
            && actual.is_some_and(|width| (width - target).abs() < 0.5)
        {
            self.state.sftp_panel_width_target = desired;
            return;
        }
        if desired.is_none() && self.state.sftp_panel_width_target.is_none() {
            return;
        }
        self.state.sftp_panel_width_target = desired;
        let Some(target) = desired else {
            return;
        };
        panels.update(cx, |state, cx| {
            state.resize_panel(index, px(target), window, cx)
        });
    }

    /// 把 SFTP 面板加宽 `delta`：挂载新列时现有面板放不下，直接向工作区分栏申请空间。
    pub(crate) fn grow_sftp_panel_width(
        &mut self,
        delta: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if delta <= 0.5 {
            return;
        }
        let Some(index) = self.sftp_resizable_index(&self.state.settings) else {
            return;
        };
        let panels = if self.state.settings.window.sftp_panel_side.is_left() {
            self.state.workspace_panels_left.clone()
        } else {
            self.state.workspace_panels.clone()
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
    fn sftp_resizable_index(&self, settings: &AppSettings) -> Option<usize> {
        if self.state.immersive {
            return None;
        }
        Some(if settings.window.sftp_panel_side.is_left() {
            1
        } else {
            2
        })
    }
}

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
