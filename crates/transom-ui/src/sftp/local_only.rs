//! 本地终端使用的本地文件面板：只渲染本地栏，不包含远端、传输队列与上传入口。
//!
//! 复用 `SftpPanelState` 与 `render_sftp_local_pane`，避免再写一套文件列表；
//! 通过 `local_only` 模式在外层裁掉远端相关内容。所有跳转、重命名、删除、
//! 过滤和快捷键仍走现有本地实现，行为与 SFTP 左栏一致。

use super::columns::ColumnWidthMemory;
use super::local_pane_view::SftpLocalPaneProps;
use super::{
    DropOverlayState, LocalQuickPaths, SftpActivePane, SftpLocalPaneInit, SftpLocalPaneState,
    SftpPanelState, SftpPanelView, SftpQueueState, SftpRemotePaneState, normalize_local_cwd,
};
use crate::app::AppShell;
use crate::app::app_settings;
use crate::divider::{DividerEdge, DividerStyled};
use crate::palette::{Palette, sftp_text_input};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::input::InputEvent;
use gpui_kit::component::resizable::ResizableState;
use gpui_kit::*;
use std::collections::HashSet;
use std::path::PathBuf;
use transom_core::Connection;
use transom_ui_icons::AppIcon;
use transom_ui_icons::UiIcon;

impl AppShell {
    pub(crate) fn open_local_file_panel(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.state.workspace.tabs.get(index) else {
            return;
        };
        let tab_id = tab.id;
        let quick_paths = LocalQuickPaths::resolve();
        // 终端上报过 OSC 7 就从当前 shell 目录打开；否则依次回落到会话配置的
        // 工作目录、Home、下载目录。本地终端默认从 Home 启动，把下载目录排在
        // 前面会让面板与终端看起来完全无关。
        let local_path = initial_local_path(tab, &quick_paths);
        log::info!(
            "打开本地文件面板，窗格: {tab_id}，初始目录: {}，终端 cwd: {:?}",
            local_path.display(),
            tab.cwd
        );
        let inputs = LocalPanelInputs::new(window, cx, &local_path);
        let local_enter_sub = cx.subscribe_in(
            &inputs.local_path_input,
            window,
            move |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.sftp_local_enter_path(tab_id, window, cx);
                }
            },
        );

        // 主连接与命令通道在挂载之前取好：面板之后自己解析目标列与投递命令。
        let main_connection = self.state.workspace.tabs[index].connection_id;
        let command_tx = Some(self.state.services.command_tx.clone());
        let state = SftpPanelState {
            main_connection,
            command_tx,
            editors: Some(self.state.editors.clone()),
            mount_candidates: Vec::new(),
            open: true,
            remote: SftpRemotePaneState::new_main(
                String::new(),
                inputs.path_input,
                inputs.name_input,
                inputs.remote_filter_input,
                false,
            ),
            local: SftpLocalPaneState::new(SftpLocalPaneInit {
                path: local_path,
                path_input: inputs.local_path_input,
                filter_input: inputs.local_filter_input,
                quick_paths,
                only: true,
                error: None,
            }),
            queue: SftpQueueState {
                tasks: Vec::new(),
                active_transfers: HashSet::new(),
                progress: None,
                resume_enabled: true,
                expanded: false,
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
            active_pane: SftpActivePane::Local,
            focus_handle: cx.focus_handle(),
            hover_preview: None,
            column_state: cx.new(|_| ResizableState::default()),
            column_widths: ColumnWidthMemory::default(),
            pending_width_adjustment: None,
            _subscriptions: vec![local_enter_sub],
        };
        // 本地单栏与 SSH 双栏共用挂载入口：创建实体、登记事件订阅与挂到
        // 窗格三者必须成对，否则关闭、新建、删除确认等意图会被静默丢弃。
        if !self.attach_sftp_panel_view(tab_id, index, state, cx) {
            return;
        }
        // 初始列表走后台读取，避免大目录或网络挂载阻塞面板打开
        self.spawn_local_dir_refresh(tab_id, None, cx);
        cx.notify();
    }
}

impl SftpPanelView {
    /// 本地终端单栏面板的内容构建；外层容器与监听由实体渲染入口统一挂载。
    pub(crate) fn render_local_only_content(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = Palette::from(&*cx);
        let local_error = self.state.local.error.clone();
        let status = self.state.queue.status.clone();
        let local_pane = self.render_sftp_local_pane(self.local_only_props(cx), &p, cx);

        let mount_btn = self.render_side_remote_mount_button(&p, cx);
        let header = self.render_sftp_panel_header(
            rust_i18n::t!("sftp.local_files_title").to_string(),
            Some(mount_btn),
            &p,
            cx,
        );
        let status_bar = if let Some(error) = local_error {
            // 本地操作失败没有任务行可查，提示保持到用户关闭或下一次成功操作；
            // 因此这里提供关闭入口，而不是让它变成清不掉的常驻文本。
            Some(
                h_flex()
                    .justify_between()
                    .items_center()
                    .px_3()
                    .py_1()
                    .text_xs()
                    .text_color(p.danger)
                    .bg(p.muted)
                    .border_color(p.divider(p.sidebar_border))
                    .child(div().flex_1().overflow_hidden().child(error))
                    .child(
                        crate::widgets::IconButton::new(
                            "local-dismiss-error",
                            AppIcon::Ui(UiIcon::CLOSE),
                            rust_i18n::t!("sftp.dismiss_error").to_string(),
                        )
                        .size(crate::widgets::IconButtonSize::Compact)
                        .render()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.dismiss_sftp_errors(cx);
                        })),
                    )
                    .divider_edge(DividerEdge::Top, p.divider_width)
                    .into_any_element(),
            )
        } else {
            status.map(|status| {
                div()
                    .px_3()
                    .py_1()
                    .text_xs()
                    .bg(p.muted)
                    .border_color(p.divider(p.sidebar_border))
                    .child(status)
                    .divider_edge(DividerEdge::Top, p.divider_width)
                    .into_any_element()
            })
        };

        let mut panel = v_flex()
            .size_full()
            .overflow_hidden()
            .bg(p.sidebar)
            .text_color(p.sidebar_foreground)
            .border_l_1()
            .border_color(p.sidebar_border)
            .child(header)
            .child(div().flex_1().w_full().overflow_hidden().child(local_pane));
        if let Some(status_bar) = status_bar {
            panel = panel.child(status_bar);
        }
        panel.into_any_element()
    }
    /// 本地单栏的渲染输入：过滤后的条目、选中集与工具栏开关。
    fn local_only_props(&self, cx: &mut Context<Self>) -> SftpLocalPaneProps {
        let state = &self.state;
        let local_filter_input = state.local.filter_input.clone();
        let show_hidden = state.local.show_hidden;
        let local_filter_query = local_filter_input
            .read(cx)
            .value()
            .trim()
            .to_ascii_lowercase();
        let hidden_patterns = app_settings(cx).sftp.panel_hidden_patterns.clone();
        let local_entries = crate::app::AppState::filter_local_entries(
            state,
            show_hidden,
            &local_filter_query,
            &hidden_patterns,
        );
        SftpLocalPaneProps {
            tab_id: self.tab_id,
            show_filter: state.local.show_filter,
            show_hidden,
            local_path_input: state.local.path_input.clone(),
            local_filter_input,
            local_entries,
            local_scroll_handle: state.local.scroll_handle.clone(),
            quick_paths: state.local.quick_paths.clone(),
            remote_available: false,
            local_selection: state.local.selection.clone(),
            local_primary: state.local.selected.clone(),
            diff_ready: false,
        }
    }
}

/// 本地面板初始目录：OSC 7 cwd → 会话工作目录 → Home → 下载目录。
fn initial_local_path(tab: &crate::app::TerminalTab, quick_paths: &LocalQuickPaths) -> PathBuf {
    let session_dir = match &tab.session.connection {
        Connection::Local(local) => local.working_dir.clone().filter(|path| path.is_dir()),
        _ => None,
    };
    tab.cwd
        .as_deref()
        .and_then(normalize_local_cwd)
        .or(session_dir)
        .or_else(|| quick_paths.home.clone())
        .or_else(|| quick_paths.downloads.clone())
        .unwrap_or_else(|| PathBuf::from("."))
}

/// 本地面板创建时用到的输入框集合。
struct LocalPanelInputs {
    path_input: Entity<gpui_kit::component::input::InputState>,
    name_input: Entity<gpui_kit::component::input::InputState>,
    remote_filter_input: Entity<gpui_kit::component::input::InputState>,
    local_path_input: Entity<gpui_kit::component::input::InputState>,
    local_filter_input: Entity<gpui_kit::component::input::InputState>,
}

impl LocalPanelInputs {
    fn new(window: &mut Window, cx: &mut Context<AppShell>, local_path: &std::path::Path) -> Self {
        let path_input = sftp_text_input(window, cx, "", rust_i18n::t!("sftp.path").as_ref());
        let name_input = sftp_text_input(window, cx, "", rust_i18n::t!("sftp.name").as_ref());
        let remote_filter_input = sftp_text_input(
            window,
            cx,
            "",
            rust_i18n::t!("sftp.sidebar.filter_remote_placeholder").as_ref(),
        );
        let local_path_input = sftp_text_input(
            window,
            cx,
            &local_path.display().to_string(),
            rust_i18n::t!("sftp.local_folder").as_ref(),
        );
        let local_filter_input = sftp_text_input(
            window,
            cx,
            "",
            rust_i18n::t!("sftp.sidebar.filter_local_placeholder").as_ref(),
        );
        Self {
            path_input,
            name_input,
            remote_filter_input,
            local_path_input,
            local_filter_input,
        }
    }
}
