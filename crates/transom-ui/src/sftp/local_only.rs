//! 本地终端使用的本地文件面板：只渲染本地栏，不包含远端、传输队列与上传入口。
//!
//! 复用 `SftpPanelState` 与 `render_sftp_local_pane`，避免再写一套文件列表；
//! 通过 `local_only` 模式在外层裁掉远端相关内容。所有跳转、重命名、删除、
//! 过滤和快捷键仍走现有本地实现，行为与 SFTP 左栏一致。

use super::columns::ColumnWidthMemory;
use super::local_pane_view::SftpLocalPaneProps;
use super::{
    DropOverlayState, LocalQuickPaths, SftpActivePane, SftpLocalPaneInit, SftpLocalPaneState,
    SftpPanelState, SftpQueueState, SftpRemotePaneState, normalize_local_cwd,
};
use crate::app::AppShell;
use crate::divider::{DividerEdge, DividerStyled};
use crate::keymap;
use crate::palette::{Palette, sftp_text_input};
use gpui_kit::base::v_flex;
use gpui_kit::component::input::InputEvent;
use gpui_kit::component::resizable::ResizableState;
use gpui_kit::*;
use std::collections::HashSet;
use std::path::PathBuf;
use transom_core::{Connection, TabId};

impl AppShell {
    pub(crate) fn open_local_file_panel(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.workspace.tabs.get(index) else {
            return;
        };
        let tab_id = tab.id;
        let quick_paths = LocalQuickPaths::resolve();
        // 终端上报过 OSC 7 就从当前 shell 目录打开；否则依次回落到会话配置的
        // 工作目录、Home、下载目录。本地终端默认从 Home 启动，把下载目录排在
        // 前面会让面板与终端看起来完全无关。
        let session_dir = match &tab.session.connection {
            Connection::Local(local) => local.working_dir.clone().filter(|path| path.is_dir()),
            _ => None,
        };
        let local_path = tab
            .cwd
            .as_deref()
            .and_then(normalize_local_cwd)
            .or(session_dir)
            .or_else(|| quick_paths.home.clone())
            .or_else(|| quick_paths.downloads.clone())
            .unwrap_or_else(|| PathBuf::from("."));
        log::info!(
            "打开本地文件面板，窗格: {tab_id}，初始目录: {}，终端 cwd: {:?}",
            local_path.display(),
            tab.cwd
        );
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
        let local_enter_sub = cx.subscribe_in(
            &local_path_input,
            window,
            move |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.sftp_local_enter_path(tab_id, window, cx);
                }
            },
        );

        let Some(pane) = self.workspace.tabs[index].session_pane_mut() else {
            return;
        };
        pane.sftp = Some(SftpPanelState {
            open: true,
            remote: SftpRemotePaneState::new_main(
                String::new(),
                path_input,
                name_input,
                remote_filter_input,
                false,
            ),
            local: SftpLocalPaneState::new(SftpLocalPaneInit {
                path: local_path,
                path_input: local_path_input,
                filter_input: local_filter_input,
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
            restored_width: None,
            _subscriptions: vec![local_enter_sub],
        });
        // 初始列表走后台读取，避免大目录或网络挂载阻塞面板打开
        self.spawn_local_dir_refresh(tab_id, None, cx);
        cx.notify();
    }

    pub(crate) fn render_local_only_panel(
        &mut self,
        tab_id: TabId,
        p: &Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(panel) = self.workspace.tab(tab_id).and_then(|tab| tab.sftp()) else {
            return div().into_any_element();
        };
        let local_path_input = panel.local.path_input.clone();
        let local_filter_input = panel.local.filter_input.clone();
        let local_scroll_handle = panel.local.scroll_handle.clone();
        let quick_paths = panel.local.quick_paths.clone();
        let show_filter = panel.local.show_filter;
        let show_hidden = panel.local.show_hidden;
        let focus_handle = panel.focus_handle.clone();
        let local_error = panel.local.error.clone();
        let status = panel.queue.status.clone();
        let local_filter_query = local_filter_input
            .read(cx)
            .value()
            .trim()
            .to_ascii_lowercase();
        let local_entries = Self::filter_local_entries(
            panel,
            show_hidden,
            &local_filter_query,
            &self.settings.sftp.panel_hidden_patterns,
        );

        let local_pane = self.render_sftp_local_pane(
            SftpLocalPaneProps {
                tab_id,
                show_filter,
                show_hidden,
                local_path_input,
                local_filter_input,
                local_entries,
                local_scroll_handle,
                quick_paths,
                remote_available: false,
            },
            p,
            cx,
        );
        let mount_btn = self.render_side_remote_mount_button(tab_id, p, cx);
        let header = self.render_sftp_panel_header(
            rust_i18n::t!("sftp.local_files_title").to_string(),
            Some(mount_btn),
            p,
            cx,
        );
        let status_bar = if let Some(error) = local_error {
            Some(
                div()
                    .px_3()
                    .py_1()
                    .text_xs()
                    .text_color(p.danger)
                    .bg(p.muted)
                    .border_color(p.divider(p.sidebar_border))
                    .child(error)
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
            .id(ElementId::Name("local-file-panel-container".into()))
            .track_focus(&focus_handle)
            .key_context(keymap::SFTP_CONTEXT)
            .on_action(cx.listener(Self::on_focus_sftp_filter_pane))
            .on_action(cx.listener(Self::on_toggle_sftp_hidden_pane))
            .on_action(cx.listener(Self::on_delete_sftp_selection))
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
}
