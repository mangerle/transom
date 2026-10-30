//! SFTP 远端目录导航、OSC 7 目录跟随与控制方法。
//!
//! 设计原理（Why）：
//! - 实现初衷：提供 SFTP 远端目录的路径跳转、向上级、回车、终端 cwd 联动与过滤切换；
//! - 核心优势：集中封装向底层通信通道投递 SftpOperation 的接口，保障状态同步一致性。

mod init;
mod local_nav;

use super::{SFTP_DIR_CACHE_TTL, normalize_local_cwd, normalize_remote_cwd, parent_remote};
use crate::app::AppShell;
use crate::keymap;
use gpui_kit::*;
use std::path::Path;
use transom_core::{SftpCommand, SftpOperation, TabId, TransferId};

impl AppShell {
    pub(crate) fn request_sftp_list(
        &mut self,
        tab_id: TabId,
        path: String,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_sftp_hover_preview(tab_id, cx);
        let Some(tab) = self.workspace.tab_mut(tab_id) else {
            return;
        };
        let Some(connection_id) = tab.connection_id else {
            return;
        };
        if let Some(panel) = tab.sftp_mut() {
            panel.remote.loading = true;
            panel.clear_error();
        }
        self.send_command(SftpCommand::Sftp {
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
        let Some(tab) = self.workspace.tab_mut(tab_id) else {
            return;
        };
        let Some(connection_id) = tab.connection_id else {
            return;
        };
        let op_id = TransferId::new();
        if let Some(panel) = tab.sftp_mut() {
            panel.queue.active_op = Some(op_id);
            if panel.queue.active_transfers.is_empty() {
                panel.queue.progress = None;
            }
            panel.clear_status();
            panel.clear_error();
        }
        self.send_command(SftpCommand::Sftp {
            tab_id,
            connection_id,
            op_id,
            op,
        });
        cx.notify();
    }

    pub(crate) fn sftp_refresh(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(path) = self
            .workspace
            .tab_mut(tab_id)
            .and_then(|tab| tab.sftp_mut())
            .map(|panel| {
                panel.remote.dir_cache.remove(&panel.remote.path);
                panel.remote.path.clone()
            })
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
            .workspace
            .tab(tab_id)
            .and_then(|tab| tab.sftp())
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
                    .local
                    .entries
                    .iter()
                    .find(|e| e.is_dir && e.name == child_name)
                    .map(|e| e.path.clone())
            });
        if let Some(local_target) = sync_local_target {
            self.sftp_local_navigate_only(tab_id, local_target, window, cx);
        }

        // 1. 即时路径更新与 SWR 短期缓存命中呈现（消除点击假死感）
        let mut update_path_input = None;
        if let Some(tab) = self.workspace.tab_mut(tab_id)
            && let Some(panel) = tab.sftp_mut()
        {
            panel.remote.path = path.clone();
            panel.remote.loading = true;
            if let Some(cached) = panel.remote.dir_cache.get(&path)
                && cached.cached_at.elapsed() < SFTP_DIR_CACHE_TTL
            {
                // 命中有效缓存：瞬间 0ms 渲染目录内容
                panel.remote.entries = cached.entries.clone();
                let valid_paths = panel
                    .remote
                    .entries
                    .iter()
                    .map(|entry| entry.path.clone())
                    .collect();
                panel.sync_remote_selection(&valid_paths);
            }
            update_path_input = Some((panel.remote.path_input.clone(), path.clone()));
        }
        if let Some((input, path_str)) = update_path_input {
            input.update(cx, |state, cx| state.set_value(path_str, window, cx));
        }

        // 2. 后台发起真实网络验证（Stale-While-Revalidate）
        self.request_sftp_list(tab_id, path, cx);
    }

    /// 远端主目录：SFTP 会话初始目录就是用户 home，请求 `.` 即可。
    pub(crate) fn sftp_remote_go_home(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sftp_navigate(tab_id, ".".to_string(), window, cx);
    }

    pub(crate) fn sftp_go_up(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((path, sync_local_parent)) = self
            .workspace
            .tab(tab_id)
            .and_then(|tab| tab.sftp())
            .map(|panel| {
                let remote_parent = parent_remote(&panel.remote.path);
                let local_parent = if panel.sync_navigation {
                    panel.local.path.parent().map(|p| p.to_path_buf())
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
            .workspace
            .tab(tab_id)
            .and_then(|tab| tab.sftp())
            .map(|panel| panel.remote.path_input.read(cx).value().trim().to_string())
        else {
            return;
        };
        if !path.is_empty() {
            self.request_sftp_list(tab_id, path, cx);
        }
    }

    pub(crate) fn sftp_toggle_filter(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(panel) = self.workspace.tab_mut(tab_id).and_then(|t| t.sftp_mut()) {
            panel.show_filter = !panel.show_filter;
            let remote_filter = panel.remote.filter_input.clone();
            let local_filter = panel.local.filter_input.clone();
            let local_only = panel.local.only;
            if panel.show_filter {
                let target = if local_only {
                    local_filter.clone()
                } else {
                    remote_filter.clone()
                };
                target.update(cx, |input, cx| {
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
        if let Some(tab_id) = self.workspace.active_tab {
            self.sftp_toggle_filter(tab_id, window, cx);
        }
    }

    pub(crate) fn sftp_toggle_hidden(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if let Some(panel) = self.workspace.tab_mut(tab_id).and_then(|t| t.sftp_mut()) {
            panel.show_hidden = !panel.show_hidden;
            panel.set_status(if panel.show_hidden {
                rust_i18n::t!("sftp.nav.hidden_shown").to_string()
            } else {
                rust_i18n::t!("sftp.nav.hidden_hidden").to_string()
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
        if let Some(tab_id) = self.workspace.active_tab {
            self.sftp_toggle_hidden(tab_id, cx);
        }
    }

    /// 终端 OSC 7 上报新目录后，按面板形态更新本地栏或远端栏。
    ///
    /// 只认 OSC 7 的裸路径，不再从窗口标题猜目录：标题的语义是"终端里在跑什么"，
    /// 可能被任意 TUI 改写；路径必须来自明确的 cwd 通道。
    pub(crate) fn sftp_sync_with_terminal_cwd(
        &mut self,
        tab_id: TabId,
        cwd: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(panel) = self.workspace.tab(tab_id).and_then(|tab| tab.sftp()) else {
            return;
        };
        if panel.local.only {
            // 目录没变就不做 is_dir：本地面板开着时每条提示符都会上报一次 cwd，
            // 网络挂载上的重复探测会拖慢 UI 线程。
            if panel.local.path.as_path() == Path::new(cwd) {
                return;
            }
            if let Some(candidate) = normalize_local_cwd(cwd)
                && candidate != panel.local.path
            {
                self.sftp_local_sync_navigate(tab_id, candidate, window, cx);
            }
            return;
        }
        let Some(candidate) = normalize_remote_cwd(cwd) else {
            return;
        };
        if candidate == panel.remote.path {
            return;
        }
        let open = panel.open;
        if let Some(panel) = self
            .workspace
            .tab_mut(tab_id)
            .and_then(|tab| tab.sftp_mut())
        {
            panel.remote.path = candidate.clone();
            panel.clear_remote_selection();
        }
        // 面板隐藏时只记路径，不为了看不见的列表消耗一次 SFTP 请求；
        // 展开时 toggle_sftp 会按当前路径补拉。
        if open {
            self.request_sftp_list(tab_id, candidate, cx);
        }
        cx.notify();
    }

    pub(crate) fn sftp_start_resize_queue(
        &mut self,
        tab_id: TabId,
        start_y: f32,
        cx: &mut Context<Self>,
    ) {
        if let Some(panel) = self.workspace.tab_mut(tab_id).and_then(|t| t.sftp_mut()) {
            panel.queue.drag_start_y = Some(start_y);
            panel.queue.drag_start_height = panel.queue.height;
            cx.notify();
        }
    }

    pub(crate) fn sftp_update_resize_queue(
        &mut self,
        tab_id: TabId,
        current_y: f32,
        cx: &mut Context<Self>,
    ) {
        if let Some(panel) = self.workspace.tab_mut(tab_id).and_then(|t| t.sftp_mut())
            && let Some(start_y) = panel.queue.drag_start_y
        {
            let delta = start_y - current_y;
            let new_height = (panel.queue.drag_start_height + delta).clamp(80.0, 480.0);
            panel.queue.height = new_height;
            cx.notify();
        }
    }

    pub(crate) fn sftp_stop_resize_queue(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if let Some(panel) = self.workspace.tab_mut(tab_id).and_then(|t| t.sftp_mut())
            && panel.queue.drag_start_y.is_some()
        {
            panel.queue.drag_start_y = None;
            cx.notify();
        }
    }
}
