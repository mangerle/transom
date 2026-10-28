//! SFTP 面板初始化与生命周期挂载。
//!
//! 设计原理（Why）：
//! - 实现初衷：为 SSH 与本地会话创建并初始化 SFTP 双栏文件浏览器状态（输入框绑定、回车监听、列表分发）；
//! - 核心优势：集中管理长生命周期 Input 实体与订阅，避免面板构造逻辑散落在各种触发动作中。

use crate::app::{AppShell, TabStatus};
use crate::keymap;
use crate::palette::sftp_text_input;
use crate::sftp::{
    DropOverlayState, LocalQuickPaths, SftpActivePane, SftpLocalPaneState, SftpPanelState,
    SftpQueueState, SftpRemotePaneState, extract_candidate_path,
};
use gpui_kit::component::VirtualListScrollHandle;
use gpui_kit::component::input::InputEvent;
use gpui_kit::*;
use std::collections::{BTreeSet, HashSet};
use std::path::PathBuf;
use transom_core::{Connection, ConnectionId, TabId};

impl AppShell {
    pub(crate) fn sftp_panel_mut(
        &mut self,
        tab_id: TabId,
        connection_id: ConnectionId,
    ) -> Option<&mut SftpPanelState> {
        let tab = self.workspace.tab_mut(tab_id)?;
        if tab.connection_id != Some(connection_id) {
            let is_side = tab
                .sftp()
                .map(|p| {
                    p.side_remotes
                        .iter()
                        .any(|s| s.connection_id == connection_id)
                })
                .unwrap_or(false);
            if !is_side {
                return None;
            }
        }
        tab.sftp_mut()
    }

    /// 将键盘焦点还给当前 SFTP 面板，供弹窗关闭后继续使用面板快捷键。
    pub(crate) fn focus_active_sftp_panel(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(panel) = self
            .workspace
            .active_tab
            .and_then(|tab_id| self.workspace.tab(tab_id))
            .and_then(|tab| tab.sftp())
        {
            window.focus(&panel.focus_handle, cx);
        }
    }

    pub(crate) fn toggle_sftp(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab_id) = self.workspace.active_tab else {
            return;
        };
        let Some(index) = self.workspace.tab_index(tab_id) else {
            return;
        };
        // `tab.sftp` 存在不等于面板展开：拖拽上传会留一份只看浮层的隐藏状态，
        // 这时 toggle 的意图是"我要看面板"，展开它而不是销毁重建队列。
        match self.workspace.tabs[index].sftp().map(|panel| panel.open) {
            Some(true) => {
                if let Some(pane) = self.workspace.tabs[index].session_pane_mut() {
                    pane.sftp = None;
                }
                cx.notify();
                return;
            }
            Some(false) => {
                let path = self.workspace.tabs[index]
                    .sftp()
                    .map(|panel| panel.remote.path.clone())
                    .unwrap_or_else(|| ".".to_string());
                if let Some(panel) = self.workspace.tabs[index].sftp_mut() {
                    panel.open = true;
                }
                // 面板接管进度显示，浮层同步退场；隐藏期间没有列表过目录，这里补上。
                self.sync_drop_overlay(tab_id, cx);
                self.spawn_local_dir_refresh(tab_id, None, cx);
                self.request_sftp_list(tab_id, path, cx);
                cx.notify();
                return;
            }
            None => {}
        }
        let is_local = matches!(
            &self.workspace.tabs[index].session.connection,
            Connection::Local(_)
        );
        let is_ssh = matches!(
            &self.workspace.tabs[index].session.connection,
            Connection::Ssh(_)
        );
        if is_local {
            self.open_local_file_panel(index, window, cx);
            return;
        }
        if !is_ssh {
            self.set_error(rust_i18n::t!("error.sftp_requires_ssh").to_string());
            cx.notify();
            return;
        }
        if !matches!(self.workspace.tabs[index].status, TabStatus::Connected) {
            self.set_error(rust_i18n::t!("error.channel_failed").to_string());
            cx.notify();
            return;
        }
        self.build_sftp_panel(tab_id, index, true, window, cx);
    }

    /// 为已连接且已就绪的 SSH 标签建立 SFTP 面板状态（输入框、回车订阅与列表）。
    pub(crate) fn build_sftp_panel(
        &mut self,
        tab_id: TabId,
        index: usize,
        open: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let quick_paths = LocalQuickPaths::resolve();
        let local_path = quick_paths
            .downloads
            .clone()
            .or_else(|| quick_paths.home.clone())
            .unwrap_or_else(|| PathBuf::from("."));
        let initial_remote_path = extract_candidate_path(&self.workspace.tabs[index].title)
            .unwrap_or_else(|| ".".to_string());
        let path_input = sftp_text_input(
            window,
            cx,
            &initial_remote_path,
            rust_i18n::t!("sftp.path").as_ref(),
        );
        let local_path_input = sftp_text_input(
            window,
            cx,
            &local_path.display().to_string(),
            rust_i18n::t!("sftp.local_folder").as_ref(),
        );
        let remote_enter_sub = cx.subscribe_in(
            &path_input,
            window,
            move |this, _, event: &InputEvent, _window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.sftp_enter_path(tab_id, cx);
                }
            },
        );
        let local_enter_sub = cx.subscribe_in(
            &local_path_input,
            window,
            move |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.sftp_local_enter_path(tab_id, window, cx);
                }
            },
        );
        let name_input = sftp_text_input(window, cx, "", rust_i18n::t!("sftp.name").as_ref());
        let remote_filter_input = sftp_text_input(window, cx, "", "过滤远程文件 (模糊匹配名称)...");
        let local_filter_input = sftp_text_input(window, cx, "", "过滤本地文件 (模糊匹配名称)...");

        let Some(pane) = self.workspace.tabs[index].session_pane_mut() else {
            return;
        };
        pane.sftp = Some(SftpPanelState {
            open,
            remote: SftpRemotePaneState {
                path: initial_remote_path.clone(),
                path_input,
                name_input,
                filter_input: remote_filter_input,
                entries: Vec::new(),
                selected: None,
                selection: BTreeSet::new(),
                anchor: None,
                scroll_handle: VirtualListScrollHandle::new(),
                collapsed: false,
                loading: true,
            },
            local: SftpLocalPaneState {
                path: local_path,
                path_input: local_path_input,
                filter_input: local_filter_input,
                entries: Vec::new(),
                quick_paths,
                only: false,
                collapsed: false,
                selected: None,
                selection: BTreeSet::new(),
                anchor: None,
                error: None,
                scroll_handle: VirtualListScrollHandle::new(),
            },
            queue: SftpQueueState {
                tasks: Vec::new(),
                active_transfers: HashSet::new(),
                active_op: None,
                progress: None,
                resume_enabled: true,
                expanded: true,
                height: 160.0,
                drag_start_y: None,
                drag_start_height: 160.0,
                status: None,
                error: None,
                status_task: None,
                drop_overlay: DropOverlayState::default(),
            },
            side_remotes: Vec::new(),
            clipboard: None,
            mount_menu_open: false,
            show_filter: false,
            show_hidden: false,
            sync_navigation: false,
            active_pane: SftpActivePane::default(),
            focus_handle: cx.focus_handle(),
            hover_preview: None,
            _subscriptions: vec![remote_enter_sub, local_enter_sub],
        });
        if open {
            // 本地栏初始列表走后台读取，避免大目录或网络挂载阻塞面板打开
            self.spawn_local_dir_refresh(tab_id, None, cx);
            self.request_sftp_list(tab_id, initial_remote_path, cx);
        }
    }

    pub(crate) fn on_toggle_sftp(
        &mut self,
        _: &keymap::ToggleSftp,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_sftp(window, cx);
    }
}
