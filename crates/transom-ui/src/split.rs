//! 标签内分屏：创建窗格、切换分屏方向与关闭窗格。

use crate::app::{AppShell, TabStatus, TerminalTab};
use crate::keymap;
use gpui_kit::*;
use std::collections::HashMap;
use transom_core::{Connection, SplitAxis, TabId};

impl AppShell {
    /// 在当前标签组内追加一个窗格。
    ///
    /// 新窗格复用当前会话配置重新建立连接（本地终端则新建 Shell）；
    /// 串口设备通常不允许独占端口被重复打开，因此明确拒绝分屏。
    pub(crate) fn split_active_pane(
        &mut self,
        axis: SplitAxis,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(active_id) = self.active_tab else {
            return;
        };
        let Some(group_index) = self.group_index_for_tab(active_id) else {
            return;
        };
        let Some(active_index) = self.tabs.iter().position(|tab| tab.id == active_id) else {
            return;
        };
        if matches!(
            self.tabs[active_index].session.connection,
            Connection::Serial(_)
        ) {
            self.error = Some(rust_i18n::t!("error.serial_no_split").to_string());
            cx.notify();
            return;
        }

        let session_id = self.tabs[active_index].session_id;
        let session = self
            .catalog
            .sessions
            .iter()
            .find(|session| session.id == session_id)
            .cloned()
            .unwrap_or_else(|| self.tabs[active_index].session.clone());
        let pane_id = TabId::new();
        self.tabs.push(TerminalTab {
            id: pane_id,
            session_id,
            session: session.clone(),
            connection_id: None,
            title: session.name.clone(),
            status: TabStatus::Connecting,
            terminal: None,
            sftp: None,
            forwards_panel: None,
            forward_statuses: HashMap::new(),
            system_stats: None,
            network_traffic: None,
            _startup_task: None,
            _subscription: None,
        });
        let group = &mut self.groups[group_index];
        group.panes.push(pane_id);
        group.active_pane = pane_id;
        group.axis = axis;
        self.active_tab = Some(pane_id);
        self.start_connection(pane_id, session, window, cx);
        cx.notify();
    }

    /// 在左右并排与上下堆叠之间切换整个标签组。
    pub(crate) fn toggle_split_axis(&mut self, cx: &mut Context<Self>) {
        let Some(index) = self.active_group_index() else {
            return;
        };
        if self.groups[index].panes.len() < 2 {
            return;
        }
        self.groups[index].axis = match self.groups[index].axis {
            SplitAxis::Horizontal => SplitAxis::Vertical,
            SplitAxis::Vertical => SplitAxis::Horizontal,
        };
        cx.notify();
    }

    pub(crate) fn on_split_right(
        &mut self,
        _: &keymap::SplitRight,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.split_active_pane(SplitAxis::Horizontal, window, cx);
    }

    pub(crate) fn on_split_down(
        &mut self,
        _: &keymap::SplitDown,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.split_active_pane(SplitAxis::Vertical, window, cx);
    }

    pub(crate) fn on_close_pane(
        &mut self,
        _: &keymap::ClosePane,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(tab_id) = self.active_tab {
            self.close_pane(tab_id, window, cx);
        }
    }
}
