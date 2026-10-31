//! 实体侧的并列远端挂载、关闭与列折叠。
//!
//! 列状态、宽度记忆与分栏状态都在实体上；挂载新列需要创建输入框与订阅，
//! 这些持有实体上下文即可完成。向工作区申请加宽属于外壳布局，经
//! `with_shell_parts` 回交。

use super::*;
use crate::sftp::columns::{SftpColumnId, plan_side_mount};
use crate::sftp::side_remote::{active_pane_after_removing, side_column_width_hint};
use crate::sftp::{SftpRemotePaneRef, SftpRemotePaneState};
use gpui_kit::component::input::InputEvent;
use gpui_kit::component::resizable::ResizableState;
use transom_core::ConnectionId;

impl SftpPanelView {
    /// 挂载并列远端会话到当前 SFTP 面板。
    pub(crate) fn mount_side_remote(
        &mut self,
        connection_id: ConnectionId,
        title: String,
        initial_path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .state
            .side_remotes
            .iter()
            .any(|side| side.connection_id == Some(connection_id))
        {
            self.state.set_status(
                rust_i18n::t!("sftp.sidebar.already_mounted", title = title.as_str()).to_string(),
            );
            cx.notify();
            return;
        }

        let mut side_state =
            SftpRemotePaneState::new_side(connection_id, title, initial_path.clone(), window, cx);
        let tab_id = self.tab_id;
        let enter_sub = cx.subscribe_in(
            &side_state.path_input,
            window,
            move |view, _, event: &InputEvent, _window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    view.enter_path_pane(tab_id, SftpRemotePaneRef::Side(connection_id), cx);
                }
            },
        );
        side_state.subscriptions.push(enter_sub);

        let width_hint = side_column_width_hint(&self.state, cx);
        let mount_plan = plan_side_mount(&self.state, width_hint, cx);
        let extra_width = mount_plan
            .as_ref()
            .map(|plan| plan.extra_width)
            .unwrap_or(0.0);
        let side_index = self.state.side_remotes.len();
        if let Some(plan) = &mount_plan {
            for (id, width) in &plan.widths {
                self.state.column_widths.remember(*id, *width);
            }
        }
        self.state
            .column_widths
            .remember(SftpColumnId::Side(side_index), width_hint);
        self.state.side_remotes.push(side_state);
        self.state.mount_menu_open = false;
        self.reset_column_state(cx);
        if extra_width > 0.5 {
            let app = self.app.clone();
            with_shell_parts(&app, cx, |shell, cx| {
                shell.grow_sftp_panel_width(extra_width, window, cx)
            });
        }
        log::info!(
            "挂载并列远端列，窗格: {tab_id}，连接: {connection_id}，初始目录: {initial_path}，建议列宽: {width_hint:.0}，面板加宽: {extra_width:.0}"
        );
        self.request_list_pane(
            tab_id,
            SftpRemotePaneRef::Side(connection_id),
            initial_path,
            cx,
        );
        cx.notify();
    }

    /// 关闭并列远端列。
    pub(crate) fn close_side_remote(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.state.side_remotes.len() {
            return;
        }
        let removed = self.state.side_remotes[index].connection_id;
        self.state.side_remotes.remove(index);
        if let Some(removed) = removed {
            self.state.active_pane = active_pane_after_removing(self.state.active_pane, removed);
        }
        self.reset_column_state(cx);
        cx.notify();
    }

    /// 切换并列远端列的展开与折叠。
    pub(crate) fn toggle_side_remote_collapse(&mut self, index: usize, cx: &mut Context<Self>) {
        self.remember_column_widths(cx);
        if let Some(side) = self.state.side_remotes.get_mut(index) {
            side.collapsed = !side.collapsed;
        }
        self.reset_column_state(cx);
        cx.notify();
    }

    /// 切换本地列的展开与折叠。
    pub(crate) fn toggle_local_collapse(&mut self, cx: &mut Context<Self>) {
        self.remember_column_widths(cx);
        self.state.local.collapsed = !self.state.local.collapsed;
        self.reset_column_state(cx);
        cx.notify();
    }

    /// 切换主远端列的展开与折叠。
    pub(crate) fn toggle_remote_collapse(&mut self, cx: &mut Context<Self>) {
        self.remember_column_widths(cx);
        self.state.remote.collapsed = !self.state.remote.collapsed;
        self.reset_column_state(cx);
        cx.notify();
    }

    /// 折叠前把当前分栏尺寸记进列宽记忆。
    pub(crate) fn remember_column_widths(&mut self, cx: &App) {
        let layout = crate::sftp::columns::column_layout(&self.state);
        let sizes = self.state.column_state.read(cx).sizes().to_vec();
        if sizes.len() != layout.len() {
            return;
        }
        for ((id, collapsed), width) in layout.iter().zip(sizes.iter()) {
            if !*collapsed {
                self.state.column_widths.remember(*id, f32::from(*width));
            }
        }
    }

    /// 折叠状态变化后重建分栏状态。
    pub(crate) fn reset_column_state(&mut self, cx: &mut Context<Self>) {
        self.state.column_state = cx.new(|_| ResizableState::default());
    }
}
