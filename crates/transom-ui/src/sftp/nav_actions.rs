//! SFTP 双栏目录导航、路径跳转、终端联动与过滤控制方法。

use super::{
    SftpActivePane, SftpPanelState, extract_candidate_path, list_local_dir, local_home,
    parent_remote,
};
use crate::app::{AppShell, TabStatus};
use crate::keymap;
use crate::palette::sftp_text_input;
use gpui_kit::component::VirtualListScrollHandle;
use gpui_kit::component::input::InputEvent;
use gpui_kit::*;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::PathBuf;
use transom_core::{AppCommand, ConnectionId, SftpOperation, TabId, TransferId};

impl AppShell {
    pub(crate) fn sftp_panel_mut(
        &mut self,
        tab_id: TabId,
        connection_id: ConnectionId,
    ) -> Option<&mut SftpPanelState> {
        let tab = self.tabs.iter_mut().find(|tab| tab.id == tab_id)?;
        if tab.connection_id != Some(connection_id) {
            return None;
        }
        tab.sftp.as_mut()
    }

    /// 将键盘焦点还给当前 SFTP 面板，供弹窗关闭后继续使用面板快捷键。
    pub(crate) fn focus_active_sftp_panel(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(panel) = self
            .active_tab
            .and_then(|tab_id| self.tabs.iter().find(|tab| tab.id == tab_id))
            .and_then(|tab| tab.sftp.as_ref())
        {
            window.focus(&panel.focus_handle, cx);
        }
    }

    pub(crate) fn toggle_sftp(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab_id) = self.active_tab else {
            return;
        };
        let Some(index) = self.tabs.iter().position(|tab| tab.id == tab_id) else {
            return;
        };
        if self.tabs[index].sftp.is_some() {
            self.tabs[index].sftp = None;
            cx.notify();
            return;
        }
        if !matches!(self.tabs[index].status, TabStatus::Connected) {
            self.error = Some(rust_i18n::t!("error.channel_failed").to_string());
            cx.notify();
            return;
        }

        let home = local_home();
        let local_path = if home.join("Downloads").is_dir() {
            home.join("Downloads")
        } else {
            home
        };
        let local_entries = list_local_dir(&local_path).unwrap_or_default();
        let initial_remote_path =
            extract_candidate_path(&self.tabs[index].title).unwrap_or_else(|| ".".to_string());
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

        self.tabs[index].sftp = Some(SftpPanelState {
            path: initial_remote_path.clone(),
            path_input,
            name_input,
            remote_filter_input,
            entries: Vec::new(),
            selected: None,
            local_path,
            local_path_input,
            local_filter_input,
            local_entries,
            local_selected: None,
            local_selection: BTreeSet::new(),
            local_anchor: None,
            local_error: None,
            local_scroll_handle: VirtualListScrollHandle::new(),
            remote_scroll_handle: VirtualListScrollHandle::new(),
            show_filter: false,
            show_hidden: false,
            sync_navigation: false,
            active_pane: SftpActivePane::default(),
            queue: Vec::new(),
            resume_enabled: true,
            queue_expanded: true,
            queue_height: 160.0,
            drag_start_y: None,
            drag_start_height: 160.0,
            focus_handle: cx.focus_handle(),
            active_op: None,
            active_transfers: HashSet::new(),
            progress: None,
            status: None,
            error: None,
            loading: true,
            pending_edit_ops: HashMap::new(),
            _subscriptions: vec![remote_enter_sub, local_enter_sub],
        });
        self.request_sftp_list(tab_id, initial_remote_path, cx);
    }

    pub(crate) fn request_sftp_list(
        &mut self,
        tab_id: TabId,
        path: String,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id) else {
            return;
        };
        let Some(connection_id) = tab.connection_id else {
            return;
        };
        if let Some(panel) = tab.sftp.as_mut() {
            panel.loading = true;
            panel.error = None;
        }
        let _ = self.services.command_tx.send(AppCommand::Sftp {
            tab_id,
            connection_id,
            op_id: TransferId::new(),
            op: SftpOperation::List { path },
        });
        cx.notify();
    }

    pub(crate) fn send_sftp_op(
        &mut self,
        tab_id: TabId,
        op: SftpOperation,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id) else {
            return;
        };
        let Some(connection_id) = tab.connection_id else {
            return;
        };
        let op_id = TransferId::new();
        if let Some(panel) = tab.sftp.as_mut() {
            panel.active_op = Some(op_id);
            if panel.active_transfers.is_empty() {
                panel.progress = None;
            }
            panel.status = None;
            panel.error = None;
        }
        let _ = self.services.command_tx.send(AppCommand::Sftp {
            tab_id,
            connection_id,
            op_id,
            op,
        });
        cx.notify();
    }

    pub(crate) fn sftp_refresh(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(path) = self
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| tab.sftp.as_ref())
            .map(|panel| panel.path.clone())
        else {
            return;
        };
        self.request_sftp_list(tab_id, path, cx);
    }

    pub(crate) fn sftp_navigate(
        &mut self,
        tab_id: TabId,
        path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let sync_local_target = self
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| tab.sftp.as_ref())
            .and_then(|panel| {
                if !panel.sync_navigation {
                    return None;
                }
                let child_name = path
                    .trim_end_matches('/')
                    .rsplit_once('/')
                    .map(|(_, n)| n)?;
                if child_name.is_empty() {
                    return None;
                }
                panel
                    .local_entries
                    .iter()
                    .find(|e| e.is_dir && e.name == child_name)
                    .map(|e| e.path.clone())
            });
        if let Some(local_target) = sync_local_target {
            self.sftp_local_navigate_only(tab_id, local_target, window, cx);
        }
        self.request_sftp_list(tab_id, path, cx);
    }

    pub(crate) fn sftp_go_up(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((path, sync_local_parent)) = self
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| tab.sftp.as_ref())
            .map(|panel| {
                let remote_parent = parent_remote(&panel.path);
                let local_parent = if panel.sync_navigation {
                    panel.local_path.parent().map(|p| p.to_path_buf())
                } else {
                    None
                };
                (remote_parent, local_parent)
            })
        else {
            return;
        };
        if let Some(local_parent) = sync_local_parent {
            self.sftp_local_navigate_only(tab_id, local_parent, window, cx);
        }
        self.request_sftp_list(tab_id, path, cx);
    }

    pub(crate) fn sftp_enter_path(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(path) = self
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| tab.sftp.as_ref())
            .map(|panel| panel.path_input.read(cx).value().trim().to_string())
        else {
            return;
        };
        if !path.is_empty() {
            self.request_sftp_list(tab_id, path, cx);
        }
    }

    pub(crate) fn sftp_local_navigate_only(
        &mut self,
        tab_id: TabId,
        target: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id) else {
            return;
        };
        let Some(panel) = tab.sftp.as_mut() else {
            return;
        };
        panel.local_path = target.clone();
        panel.local_entries = list_local_dir(&target).unwrap_or_default();
        panel.clear_local_selection();
        panel.local_error = None;
        let input = panel.local_path_input.clone();
        input.update(cx, |state, cx| {
            state.set_value(target.display().to_string(), window, cx);
        });
        cx.notify();
    }

    pub(crate) fn sftp_local_navigate(
        &mut self,
        tab_id: TabId,
        target: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let sync_remote_target = self
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| tab.sftp.as_ref())
            .and_then(|panel| {
                if !panel.sync_navigation {
                    return None;
                }
                let child_name = target.file_name()?.to_str()?;
                panel
                    .entries
                    .iter()
                    .find(|e| e.is_dir && e.name == child_name)
                    .map(|e| e.path.clone())
            });
        self.sftp_local_navigate_only(tab_id, target, window, cx);
        if let Some(remote_path) = sync_remote_target {
            self.request_sftp_list(tab_id, remote_path, cx);
        }
    }

    pub(crate) fn sftp_local_go_up(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (parent, sync_remote_parent) = self
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| tab.sftp.as_ref())
            .map(|panel| {
                let local_parent = panel.local_path.parent().map(|p| p.to_path_buf());
                let remote_parent = if panel.sync_navigation {
                    Some(parent_remote(&panel.path))
                } else {
                    None
                };
                (local_parent, remote_parent)
            })
            .unwrap_or((None, None));
        if let Some(parent) = parent {
            self.sftp_local_navigate_only(tab_id, parent, window, cx);
            if let Some(remote_parent) = sync_remote_parent {
                self.request_sftp_list(tab_id, remote_parent, cx);
            }
        }
    }

    pub(crate) fn sftp_local_refresh(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if let Some(panel) = self
            .tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| tab.sftp.as_mut())
        {
            let entries = list_local_dir(&panel.local_path).unwrap_or_default();
            let valid_paths = entries.iter().map(|entry| entry.path.clone()).collect();
            panel.local_entries = entries;
            panel.sync_local_selection(&valid_paths);
            panel.local_error = None;
            cx.notify();
        }
    }

    pub(crate) fn sftp_local_enter_path(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = self
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| tab.sftp.as_ref())
            .map(|panel| PathBuf::from(panel.local_path_input.read(cx).value().trim()));
        if let Some(target) = target
            && target.is_dir()
        {
            self.sftp_local_navigate(tab_id, target, window, cx);
        }
    }

    pub(crate) fn sftp_open_local_file_manager(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(local_path) = self
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| tab.sftp.as_ref())
            .map(|panel| panel.local_path.clone())
        else {
            return;
        };
        #[cfg(target_os = "macos")]
        let program = "open";
        #[cfg(target_os = "windows")]
        let program = "explorer";
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let program = "xdg-open";

        let path_for_thread = local_path.clone();
        let _ = std::thread::Builder::new()
            .name("transom-open-file-manager".into())
            .spawn(move || {
                if let Ok(mut child) = std::process::Command::new(program)
                    .arg(&path_for_thread)
                    .spawn()
                {
                    let _ = child.wait();
                }
            });

        if let Some(panel) = self
            .tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
            .and_then(|tab| tab.sftp.as_mut())
        {
            panel.status = Some(format!(
                "已在系统文件管理器中打开: {}",
                local_path.display()
            ));
            cx.notify();
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

    pub(crate) fn sftp_toggle_filter(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(panel) = self
            .tabs
            .iter_mut()
            .find(|t| t.id == tab_id)
            .and_then(|t| t.sftp.as_mut())
        {
            panel.show_filter = !panel.show_filter;
            let remote_filter = panel.remote_filter_input.clone();
            let local_filter = panel.local_filter_input.clone();
            if panel.show_filter {
                remote_filter.update(cx, |input, cx| {
                    input.focus(window, cx);
                });
            } else {
                remote_filter.update(cx, |input, cx| {
                    input.set_value("", window, cx);
                });
                local_filter.update(cx, |input, cx| {
                    input.set_value("", window, cx);
                });
            }
            cx.notify();
        }
    }

    pub(crate) fn on_focus_sftp_filter(
        &mut self,
        _: &keymap::FocusSftpFilter,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(tab_id) = self.active_tab {
            self.sftp_toggle_filter(tab_id, window, cx);
        }
    }

    pub(crate) fn sftp_toggle_hidden(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if let Some(panel) = self
            .tabs
            .iter_mut()
            .find(|t| t.id == tab_id)
            .and_then(|t| t.sftp.as_mut())
        {
            panel.show_hidden = !panel.show_hidden;
            panel.status = Some(if panel.show_hidden {
                "已显示隐藏文件".to_string()
            } else {
                "已隐藏点号文件".to_string()
            });
            cx.notify();
        }
    }

    pub(crate) fn on_toggle_sftp_hidden(
        &mut self,
        _: &keymap::ToggleSftpHidden,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(tab_id) = self.active_tab {
            self.sftp_toggle_hidden(tab_id, cx);
        }
    }

    pub(crate) fn sftp_sync_with_terminal_title(
        &mut self,
        tab_id: TabId,
        title: &str,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) else {
            return;
        };
        let Some(panel) = tab.sftp.as_mut() else {
            return;
        };
        if let Some(candidate) = extract_candidate_path(title)
            && candidate != panel.path
        {
            panel.path = candidate.clone();
            panel.selected = None;
            self.request_sftp_list(tab_id, candidate, cx);
            cx.notify();
        }
    }

    pub(crate) fn sftp_start_resize_queue(
        &mut self,
        tab_id: TabId,
        start_y: f32,
        cx: &mut Context<Self>,
    ) {
        if let Some(panel) = self
            .tabs
            .iter_mut()
            .find(|t| t.id == tab_id)
            .and_then(|t| t.sftp.as_mut())
        {
            panel.drag_start_y = Some(start_y);
            panel.drag_start_height = panel.queue_height;
            cx.notify();
        }
    }

    pub(crate) fn sftp_update_resize_queue(
        &mut self,
        tab_id: TabId,
        current_y: f32,
        cx: &mut Context<Self>,
    ) {
        if let Some(panel) = self
            .tabs
            .iter_mut()
            .find(|t| t.id == tab_id)
            .and_then(|t| t.sftp.as_mut())
            && let Some(start_y) = panel.drag_start_y
        {
            let delta = start_y - current_y;
            let new_height = (panel.drag_start_height + delta).clamp(80.0, 480.0);
            panel.queue_height = new_height;
            cx.notify();
        }
    }

    pub(crate) fn sftp_stop_resize_queue(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if let Some(panel) = self
            .tabs
            .iter_mut()
            .find(|t| t.id == tab_id)
            .and_then(|t| t.sftp.as_mut())
            && panel.drag_start_y.is_some()
        {
            panel.drag_start_y = None;
            cx.notify();
        }
    }
}
