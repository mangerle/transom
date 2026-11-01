//! SFTP 面板初始化与生命周期挂载。
//!
//! 设计原理（Why）：
//! - 实现初衷：为 SSH 与本地会话创建并初始化 SFTP 双栏文件浏览器状态（输入框绑定、回车监听、列表分发）；
//! - 核心优势：集中管理长生命周期 Input 实体与订阅，避免面板构造逻辑散落在各种触发动作中。

use crate::app::{AppShell, TabStatus};
use crate::keymap;
use crate::palette::sftp_text_input;
use crate::sftp::columns::ColumnWidthMemory;
use crate::sftp::{
    DropOverlayState, LocalQuickPaths, SftpActivePane, SftpLocalPaneInit, SftpLocalPaneState,
    SftpPanelState, SftpPanelView, SftpQueueState, SftpRemotePaneRef, SftpRemotePaneState,
    normalize_remote_cwd,
};
use gpui_kit::component::input::InputEvent;
use gpui_kit::component::resizable::ResizableState;
use gpui_kit::*;
use std::collections::HashSet;
use std::path::PathBuf;
use transom_core::{Connection, TabId};

impl AppShell {
    /// 处理面板发来的意图事件。
    ///
    /// 只使用事件携带的数据，不回读面板：事件派发阶段面板仍处于租借中。
    fn on_sftp_panel_event(&mut self, event: &crate::sftp::SftpPanelEvent, cx: &mut Context<Self>) {
        use crate::sftp::SftpPanelEvent;
        match event {
            SftpPanelEvent::SendCommand(command) => {
                self.state.send_command(command.clone());
                cx.notify();
            }
            SftpPanelEvent::OpenChmodModal {
                tab_id,
                connection_id,
                remote_path,
                file_name,
                permissions,
                octal_input,
            } => {
                self.state.sftp_modals.chmod = Some(crate::sftp::SftpChmodModalState {
                    tab_id: *tab_id,
                    connection_id: *connection_id,
                    remote_path: remote_path.clone(),
                    file_name: file_name.clone(),
                    permissions: *permissions,
                    octal_input: octal_input.clone(),
                    error: None,
                });
                cx.notify();
            }
            SftpPanelEvent::PersistSettings { status } => {
                let snapshot = crate::app::app_settings(cx).clone();
                if let Err(error) = self.state.persist_settings(&snapshot) {
                    self.state
                        .set_error(crate::palette::localized_error(&error));
                }
                self.state.status = Some(status.clone());
                cx.notify();
            }
            SftpPanelEvent::ConfirmLocalDelete { tab_id, paths } => {
                self.state.sftp_modals.confirm = Some(crate::sftp::SftpConfirmModalState {
                    action: crate::sftp::SftpPendingDelete::Local {
                        tab_id: *tab_id,
                        paths: paths.clone(),
                    },
                    title: rust_i18n::t!("sftp.local_async.delete_title").to_string(),
                    description: rust_i18n::t!("sftp.local_async.delete_body", count = paths.len())
                        .to_string(),
                });
                cx.notify();
            }
            SftpPanelEvent::ConfirmRemoteDelete {
                tab_id,
                connection_id,
                targets,
            } => {
                self.state.sftp_modals.confirm = Some(crate::sftp::SftpConfirmModalState {
                    action: crate::sftp::SftpPendingDelete::Remote {
                        tab_id: *tab_id,
                        connection_id: *connection_id,
                        targets: targets.clone(),
                    },
                    title: rust_i18n::t!("sftp.file_ops.delete_remote_title").to_string(),
                    description: rust_i18n::t!(
                        "sftp.file_ops.delete_remote_body",
                        count = targets.len()
                    )
                    .to_string(),
                });
                cx.notify();
            }
        }
    }

    /// 将键盘焦点还给当前 SFTP 面板，供弹窗关闭后继续使用面板快捷键。
    pub(crate) fn focus_active_sftp_panel(&self, window: &mut Window, cx: &mut Context<Self>) {
        let focus_handle = self
            .state
            .workspace
            .active_tab
            .and_then(|tab_id| self.sftp_state(tab_id, cx))
            .map(|panel| panel.focus_handle.clone());
        if let Some(focus_handle) = focus_handle {
            window.focus(&focus_handle, cx);
        }
    }

    pub(crate) fn toggle_sftp(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab_id) = self.state.workspace.active_tab else {
            return;
        };
        let Some(index) = self.state.workspace.tab_index(tab_id) else {
            return;
        };
        // `tab.sftp` 存在不等于面板展开：拖拽上传会留一份只看浮层的隐藏状态，
        // 这时 toggle 的意图是"我要看面板"，展开它而不是销毁重建队列。
        match self.sftp_state(tab_id, cx).map(|panel| panel.open) {
            Some(true) => {
                self.close_sftp_panel(tab_id);
                cx.notify();
                return;
            }
            Some(false) => {
                let path = self
                    .sftp_state(tab_id, cx)
                    .map(|panel| panel.remote.path.clone())
                    .unwrap_or_else(|| ".".to_string());
                if let Some(mut view) = self.sftp_view_mut(tab_id, cx) {
                    view.state.open = true;
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
            &self.state.workspace.tabs[index].session.connection,
            Connection::Local(_)
        );
        let is_ssh = matches!(
            &self.state.workspace.tabs[index].session.connection,
            Connection::Ssh(_)
        );
        if is_local {
            self.open_local_file_panel(index, window, cx);
            return;
        }
        if !is_ssh {
            self.state
                .set_error(rust_i18n::t!("error.sftp_requires_ssh").to_string());
            cx.notify();
            return;
        }
        if !matches!(
            self.state.workspace.tabs[index].status,
            TabStatus::Connected
        ) {
            self.state
                .set_error(rust_i18n::t!("error.channel_failed").to_string());
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
        // 初始远端目录优先用终端 OSC 7 上报的 cwd；还没上报时回落到远端 Home。
        let initial_remote_path = self.state.workspace.tabs[index]
            .cwd
            .as_deref()
            .and_then(normalize_remote_cwd)
            .unwrap_or_else(|| ".".to_string());
        log::info!(
            "打开远端文件面板，窗格: {tab_id}，初始目录: {initial_remote_path}，终端 cwd: {:?}",
            self.state.workspace.tabs[index].cwd
        );
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
                    this.sftp_enter_path_pane(tab_id, SftpRemotePaneRef::Main, cx);
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
        let remote_filter_input = sftp_text_input(
            window,
            cx,
            "",
            rust_i18n::t!("sftp.sidebar.filter_remote_placeholder").as_ref(),
        );
        let local_filter_input = sftp_text_input(
            window,
            cx,
            "",
            rust_i18n::t!("sftp.sidebar.filter_local_placeholder").as_ref(),
        );

        // 主连接在借用窗格之前取好：面板之后自己解析目标列，不再反查外壳。
        let main_connection = self.state.workspace.tabs[index].connection_id;
        let Some(pane) = self.state.workspace.tabs[index].session_pane_mut() else {
            return;
        };
        let state = SftpPanelState {
            main_connection,
            open,
            remote: SftpRemotePaneState::new_main(
                initial_remote_path.clone(),
                path_input,
                name_input,
                remote_filter_input,
                true,
            ),
            local: SftpLocalPaneState::new(SftpLocalPaneInit {
                path: local_path,
                path_input: local_path_input,
                filter_input: local_filter_input,
                quick_paths,
                only: false,
                error: None,
            }),
            queue: SftpQueueState {
                tasks: Vec::new(),
                active_transfers: HashSet::new(),
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
            sync_navigation: false,
            active_pane: SftpActivePane::default(),
            focus_handle: cx.focus_handle(),
            hover_preview: None,
            // 分栏状态由面板自己持有：折叠期间分栏组不渲染，
            // 窗口的 keyed state 会被回收，列宽必须另有去处。
            column_state: cx.new(|_| ResizableState::default()),
            column_widths: ColumnWidthMemory::default(),
            restored_width: None,
            _subscriptions: vec![remote_enter_sub, local_enter_sub],
        };
        let app = cx.entity().downgrade();
        let view = cx.new(|_| {
            let mut view = SftpPanelView::new(state, tab_id);
            view.attach(app);
            view
        });
        let subscription = cx.subscribe(
            &view,
            |shell, _view, event: &crate::sftp::SftpPanelEvent, cx| {
                shell.on_sftp_panel_event(event, cx);
            },
        );
        self.state
            .tasks
            .sftp_panel_subscriptions
            .insert(tab_id, subscription);
        pane.sftp_view = Some(view);
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
