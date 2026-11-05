//! SFTP 多列分栏的 UI 测量、折叠与拖拽状态。

use super::width::COLLAPSED_COLUMN_WIDTH;
use super::*;
use crate::app::app_settings;
use crate::sftp::entity::SftpPanelView;
use transom_core::AppSettings;

impl AppShell {
    /// 处理 SFTP 面板在列折叠/展开时产生的尺寸调整请求。
    ///
    /// 设计原理（Why）：折叠动作在事件或菜单回调中触发，此时无法获取可变窗口句柄；
    /// 将调整意图（`pending_width_adjustment`）记录在面板状态中，由渲染周期的本方法
    /// 一次性执行 `resize_panel` 并清空。
    /// 消费后不再对面板施加持续的每帧硬性重置，从而完全保留用户手动拖拽调整面板宽度的能力。
    pub(crate) fn sync_sftp_panel_width(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab_id) = self.state.workspace.active_tab else {
            return;
        };
        let adjustment = self
            .sftp_view_mut(tab_id, cx)
            .and_then(|mut panel| panel.state.pending_width_adjustment.take());
        let Some(adjustment) = adjustment else {
            return;
        };
        let Some(panel) = self.sftp_state(tab_id, cx) else {
            return;
        };
        if !panel.open {
            return;
        }
        let Some(index) = self.sftp_resizable_index(app_settings(cx)) else {
            return;
        };
        let panels = if app_settings(cx).window.sftp_panel_side.is_left() {
            self.state.workspace_panels_left.clone()
        } else {
            self.state.workspace_panels.clone()
        };
        let current_width = panels
            .read(cx)
            .sizes()
            .get(index)
            .copied()
            .map(|width| width.as_f32())
            .unwrap_or(360.0);

        let target = match adjustment {
            SftpWidthAdjustment::Exact(w) => w,
            SftpWidthAdjustment::Delta(d) => current_width + d,
        };
        let min_w = panel.min_content_width();
        let target = target.max(min_w);

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
        let Some(index) = self.sftp_resizable_index(app_settings(cx)) else {
            return;
        };
        let panels = if app_settings(cx).window.sftp_panel_side.is_left() {
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
