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
    /// 打开类事件：直接写入对应弹窗状态；返回是否已处理。
    fn try_open_sftp_modal(
        &mut self,
        event: &crate::sftp::SftpPanelEvent,
        cx: &mut Context<Self>,
    ) -> bool {
        use crate::sftp::SftpPanelEvent;
        match event {
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
            SftpPanelEvent::OpenSearchModal(state) => {
                self.state.sftp_modals.search = Some((**state).clone());
                cx.notify();
            }
            SftpPanelEvent::ShowProperties { scope, properties } => {
                self.state.sftp_modals.properties = Some(crate::sftp::SftpPropertiesModalState {
                    scope: scope.clone(),
                    properties: properties.clone(),
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
            _ => return false,
        }
        true
    }

    fn on_sftp_panel_event(&mut self, event: &crate::sftp::SftpPanelEvent, cx: &mut Context<Self>) {
        use crate::sftp::SftpPanelEvent;
        if self.try_open_sftp_modal(event, cx) {
            return;
        }
        match event {
            SftpPanelEvent::SendCommand(command) => {
                self.state.send_command(command.clone());
                cx.notify();
            }
            SftpPanelEvent::OpenEditor(_)
            | SftpPanelEvent::OpenInputModal(_)
            | SftpPanelEvent::GrowPanelWidth(_) => {
                // 需要窗口的动作推到渲染期执行：事件派发时没有 Window。
                self.state.window_requests.push_sftp(event.clone());
            }
            SftpPanelEvent::OpenSyncModal(state) => {
                self.state.sftp_modals.sync = Some((**state).clone());
                cx.notify();
            }
            SftpPanelEvent::OpenFilterModal { textarea } => {
                self.state.sftp_modals.filter = Some(crate::sftp::SftpFilterModalState {
                    textarea: textarea.clone(),
                    error: None,
                });
                cx.notify();
            }
            SftpPanelEvent::ClosePanel { tab_id } => {
                self.state.close_sftp_panel(*tab_id);
                cx.notify();
            }
            SftpPanelEvent::OpenLocalDiff { tab_id, target } => {
                self.open_local_diff_with_target(*tab_id, target.clone(), cx);
            }
            SftpPanelEvent::SetStatus(message) => {
                self.state.status = Some(message.clone());
                cx.notify();
            }
            // 需要弹窗状态的分支已由 try_open_sftp_modal 处理。
            _ => {}
        }
    }

    /// 创建面板实体、登记它发往外壳的意图事件订阅，并挂到窗格上。
    ///
    /// 设计原理（Why）：创建、订阅与挂载必须成对发生。面板的跨域动作
    /// （关闭面板、新建/重命名、删除确认、窗口类请求）全部走
    /// [`SftpPanelEvent`] 上报，漏登记订阅时事件被静默丢弃，表现为按钮
    /// 点击后毫无反应。把三件事收进唯一入口后，SSH 双栏与本地单栏不会
    /// 因为各写一份创建代码而出现行为分叉。
    ///
    /// 返回 `false` 表示窗格已不存在，此时不会创建面板实体。
    ///
    /// [`SftpPanelEvent`]: crate::sftp::SftpPanelEvent
    pub(crate) fn attach_sftp_panel_view(
        &mut self,
        tab_id: TabId,
        index: usize,
        state: SftpPanelState,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(pane) = self
            .state
            .workspace
            .tabs
            .get_mut(index)
            .and_then(|tab| tab.session_pane_mut())
        else {
            return false;
        };
        let view = cx.new(|_| SftpPanelView::new(state, tab_id));
        let subscription = cx.subscribe(
            &view,
            |shell, _view, event: &crate::sftp::SftpPanelEvent, cx| {
                shell.on_sftp_panel_event(event, cx);
            },
        );
        self.state
            .tasks
            .subscriptions
            .sftp
            .insert(tab_id, subscription);
        pane.sftp_view = Some(view);
        true
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
                self.state.close_sftp_panel(tab_id);
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
    /// 面板初始路径：本地快捷目录与远端 OSC 7 cwd（回落到 `.`）。
    fn sftp_initial_paths(&self, index: usize) -> (LocalQuickPaths, PathBuf, String) {
        let tab_id = self.state.workspace.tabs[index].id;
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
        (quick_paths, local_path, initial_remote_path)
    }

    /// 构造面板状态：输入控件、订阅与初始路径一次性装入。
    fn build_sftp_state(
        &mut self,
        input: SftpStateInput,
        cx: &mut Context<Self>,
    ) -> SftpPanelState {
        let SftpStateInput {
            main_connection,
            command_tx,
            inputs,
            open,
            initial_remote_path,
            local_path,
            quick_paths,
            remote_enter_sub,
            local_enter_sub,
        } = input;
        SftpPanelState {
            main_connection,
            command_tx,
            editors: Some(self.state.editors.clone()),
            mount_candidates: Vec::new(),
            open,
            remote: SftpRemotePaneState::new_main(
                initial_remote_path.clone(),
                inputs.path_input,
                inputs.name_input,
                inputs.remote_filter_input,
                true,
            ),
            local: SftpLocalPaneState::new(SftpLocalPaneInit {
                path: local_path,
                path_input: inputs.local_path_input,
                filter_input: inputs.local_filter_input,
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
                error_task: None,
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
            pending_width_adjustment: None,
            _subscriptions: vec![remote_enter_sub, local_enter_sub],
        }
    }

    pub(crate) fn build_sftp_panel(
        &mut self,
        tab_id: TabId,
        index: usize,
        open: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (quick_paths, local_path, initial_remote_path) = self.sftp_initial_paths(index);
        let inputs = SftpPanelInputs::new(window, cx, &initial_remote_path, &local_path);
        let remote_enter_sub = cx.subscribe_in(
            &inputs.path_input,
            window,
            move |this, _, event: &InputEvent, _window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.sftp_enter_path_pane(tab_id, SftpRemotePaneRef::Main, cx);
                }
            },
        );
        let local_enter_sub = cx.subscribe_in(
            &inputs.local_path_input,
            window,
            move |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.sftp_local_enter_path(tab_id, window, cx);
                }
            },
        );
        // 主连接与命令通道在借用窗格之前取好：面板之后自己解析目标列与投递命令。
        let main_connection = self.state.workspace.tabs[index].connection_id;
        let command_tx = Some(self.state.services.command_tx.clone());
        let state = self.build_sftp_state(
            SftpStateInput {
                main_connection,
                command_tx,
                inputs,
                open,
                initial_remote_path: initial_remote_path.clone(),
                local_path,
                quick_paths,
                remote_enter_sub,
                local_enter_sub,
            },
            cx,
        );
        if !self.attach_sftp_panel_view(tab_id, index, state, cx) {
            return;
        }
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

/// 面板状态构造输入：连接、输入控件、订阅与初始路径。
struct SftpStateInput {
    main_connection: Option<transom_core::ConnectionId>,
    command_tx: Option<flume::Sender<transom_core::AppCommand>>,
    inputs: SftpPanelInputs,
    open: bool,
    initial_remote_path: String,
    local_path: std::path::PathBuf,
    quick_paths: LocalQuickPaths,
    remote_enter_sub: Subscription,
    local_enter_sub: Subscription,
}

/// SFTP 面板创建时用到的输入框集合。
struct SftpPanelInputs {
    path_input: Entity<gpui_kit::component::input::InputState>,
    name_input: Entity<gpui_kit::component::input::InputState>,
    remote_filter_input: Entity<gpui_kit::component::input::InputState>,
    local_path_input: Entity<gpui_kit::component::input::InputState>,
    local_filter_input: Entity<gpui_kit::component::input::InputState>,
}

impl SftpPanelInputs {
    fn new(
        window: &mut Window,
        cx: &mut Context<crate::app::AppShell>,
        remote_path: &str,
        local_path: &std::path::Path,
    ) -> Self {
        Self {
            path_input: sftp_text_input(
                window,
                cx,
                remote_path,
                rust_i18n::t!("sftp.path").as_ref(),
            ),
            name_input: sftp_text_input(window, cx, "", rust_i18n::t!("sftp.name").as_ref()),
            remote_filter_input: sftp_text_input(
                window,
                cx,
                "",
                rust_i18n::t!("sftp.sidebar.filter_remote_placeholder").as_ref(),
            ),
            local_path_input: sftp_text_input(
                window,
                cx,
                &local_path.display().to_string(),
                rust_i18n::t!("sftp.local_folder").as_ref(),
            ),
            local_filter_input: sftp_text_input(
                window,
                cx,
                "",
                rust_i18n::t!("sftp.sidebar.filter_local_placeholder").as_ref(),
            ),
        }
    }
}
