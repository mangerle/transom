//! 本地终端使用的本地文件面板：只渲染本地栏，不包含远端、传输队列与上传入口。
//!
//! 复用 `SftpPanelState` 与 `render_sftp_local_pane`，避免再写一套文件列表；
//! 通过 `local_only` 模式在外层裁掉远端相关内容。所有跳转、重命名、删除、
//! 过滤和快捷键仍走现有本地实现，行为与 SFTP 左栏一致。

use super::local_pane_view::SftpLocalPaneProps;
use super::{
    DropOverlayState, LocalQuickPaths, SftpActivePane, SftpLocalPaneState, SftpPanelState,
    SftpQueueState, SftpRemotePaneState, fuzzy_match_name,
};
use crate::app::AppShell;
use crate::keymap;
use crate::palette::{Palette, sftp_text_input};
use gpui_kit::base::v_flex;
use gpui_kit::component::VirtualListScrollHandle;
use gpui_kit::component::input::InputEvent;
use gpui_kit::*;
use std::collections::{BTreeSet, HashSet};
use std::path::PathBuf;
use transom_core::TabId;

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
        let local_path = quick_paths
            .downloads
            .clone()
            .or_else(|| quick_paths.home.clone())
            .unwrap_or_else(|| PathBuf::from("."));
        let local_entries = Vec::new();
        let local_error = None;

        let path_input = sftp_text_input(window, cx, "", rust_i18n::t!("sftp.path").as_ref());
        let name_input = sftp_text_input(window, cx, "", rust_i18n::t!("sftp.name").as_ref());
        let remote_filter_input = sftp_text_input(window, cx, "", "过滤远程文件 (模糊匹配名称)...");
        let local_path_input = sftp_text_input(
            window,
            cx,
            &local_path.display().to_string(),
            rust_i18n::t!("sftp.local_folder").as_ref(),
        );
        let local_filter_input = sftp_text_input(window, cx, "", "过滤本地文件 (模糊匹配名称)...");
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
            remote: SftpRemotePaneState {
                path: String::new(),
                path_input,
                name_input,
                filter_input: remote_filter_input,
                entries: Vec::new(),
                selected: None,
                selection: BTreeSet::new(),
                anchor: None,
                scroll_handle: VirtualListScrollHandle::new(),
                collapsed: false,
                loading: false,
                dir_cache: std::collections::HashMap::new(),
            },
            local: SftpLocalPaneState {
                path: local_path,
                path_input: local_path_input,
                filter_input: local_filter_input,
                entries: local_entries,
                quick_paths,
                only: true,
                collapsed: false,
                selected: None,
                selection: BTreeSet::new(),
                anchor: None,
                error: local_error,
                scroll_handle: VirtualListScrollHandle::new(),
            },
            queue: SftpQueueState {
                tasks: Vec::new(),
                active_transfers: HashSet::new(),
                active_op: None,
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
            show_filter: false,
            show_hidden: false,
            sync_navigation: false,
            active_pane: SftpActivePane::Local,
            focus_handle: cx.focus_handle(),
            hover_preview: None,
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
        let show_filter = panel.show_filter;
        let show_hidden = panel.show_hidden;
        let focus_handle = panel.focus_handle.clone();
        let local_error = panel.local.error.clone();
        let status = panel.queue.status.clone();
        let local_filter_query = local_filter_input
            .read(cx)
            .value()
            .trim()
            .to_ascii_lowercase();
        let local_entries = panel
            .local
            .entries
            .iter()
            .filter(|entry| {
                (show_hidden || !entry.name.starts_with('.'))
                    && (local_filter_query.is_empty()
                        || fuzzy_match_name(&entry.name, &local_filter_query))
            })
            .cloned()
            .collect();

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
                    .border_t_1()
                    .border_color(p.sidebar_border)
                    .child(error)
                    .into_any_element(),
            )
        } else {
            status.map(|status| {
                div()
                    .px_3()
                    .py_1()
                    .text_xs()
                    .bg(p.muted)
                    .border_t_1()
                    .border_color(p.sidebar_border)
                    .child(status)
                    .into_any_element()
            })
        };

        let mut panel = v_flex()
            .id(ElementId::Name("local-file-panel-container".into()))
            .track_focus(&focus_handle)
            .key_context(keymap::SFTP_CONTEXT)
            .on_action(cx.listener(Self::on_focus_sftp_filter))
            .on_action(cx.listener(Self::on_toggle_sftp_hidden))
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
