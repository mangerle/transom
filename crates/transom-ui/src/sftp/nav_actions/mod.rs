//! SFTP 远端目录导航、OSC 7 目录跟随与控制方法。
//!
//! 设计原理（Why）：
//! - 实现初衷：提供 SFTP 远端目录的路径跳转、向上级、回车、终端 cwd 联动与过滤切换；
//! - 核心优势：集中封装向底层通信通道投递 SftpOperation 的接口，保障状态同步一致性。

mod init;
mod local_nav;

use super::entity::{SftpPanelView, read_shell_parts, with_shell_parts};
use super::{SftpRemotePaneRef, normalize_local_cwd, normalize_remote_cwd};
use crate::app::AppShell;
use gpui_kit::*;
use std::path::Path;
use transom_core::{SftpCommand, SftpOperation, TabId, TransferId};

impl SftpPanelView {
    /// 向目标远端列发起目录列举：主远端与并列远端共用同一套加载态与错误清理。
    pub(crate) fn request_list_pane(
        &mut self,
        tab_id: TabId,
        target: SftpRemotePaneRef,
        path: String,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_hover_preview(cx);
        let app = self.app.clone();
        let Some(main_connection) = read_shell_parts(&app, cx, |shell| {
            shell
                .state
                .workspace
                .tab(tab_id)
                .and_then(|tab| tab.connection_id)
        })
        .flatten() else {
            log::warn!("远端目录请求找不到目标列，窗格: {tab_id}，目标: {target:?}，路径: {path}");
            return;
        };
        let Some(connection_id) = self.state.remote_connection(target, Some(main_connection))
        else {
            log::warn!("远端目录请求找不到目标列，窗格: {tab_id}，目标: {target:?}，路径: {path}");
            return;
        };
        log::debug!(
            "请求远端目录列表，窗格: {tab_id}，目标: {target:?}，连接: {connection_id}，路径: {path}"
        );
        if let Some(pane) = self.state.remote_pane_mut(target) {
            pane.loading = true;
            pane.error = None;
        }
        with_shell_parts(&app, cx, |shell, _| {
            shell.state.send_command(SftpCommand::Sftp {
                tab_id,
                connection_id,
                op_id: TransferId::new(),
                op: SftpOperation::List { path },
            });
        });
        cx.notify();
    }

    pub(crate) fn request_list(&mut self, tab_id: TabId, path: String, cx: &mut Context<Self>) {
        self.request_list_pane(tab_id, SftpRemotePaneRef::Main, path, cx);
    }

    /// 终端 OSC 7 上报新目录后，按面板形态更新本地栏或远端栏。
    pub(crate) fn sync_with_terminal_cwd(
        &mut self,
        tab_id: TabId,
        cwd: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.state.local.only {
            let local_path = self.state.local.path.clone();
            // 目录没变就不做 is_dir：本地面板开着时每条提示符都会上报一次 cwd，
            // 网络挂载上的重复探测会拖慢 UI 线程。
            if local_path.as_path() == Path::new(cwd) {
                return;
            }
            if let Some(candidate) = normalize_local_cwd(cwd)
                && candidate != local_path
            {
                self.local_sync_navigate(tab_id, candidate, window, cx);
            }
            return;
        }
        let Some(candidate) = normalize_remote_cwd(cwd) else {
            return;
        };
        if candidate == self.state.remote.path {
            return;
        }
        let open = self.state.open;
        self.state.remote.path = candidate.clone();
        self.state.clear_remote_selection();
        // 面板隐藏时只记路径，不为了看不见的列表消耗一次 SFTP 请求；
        // 展开时 toggle_sftp 会按当前路径补拉。
        if open {
            self.request_list(tab_id, candidate, cx);
        }
        cx.notify();
    }

    pub(crate) fn start_resize_queue(&mut self, start_y: f32, cx: &mut Context<Self>) {
        self.state.queue.drag_start_y = Some(start_y);
        self.state.queue.drag_start_height = self.state.queue.height;
        cx.notify();
    }

    pub(crate) fn update_resize_queue(&mut self, current_y: f32, cx: &mut Context<Self>) {
        if let Some(start_y) = self.state.queue.drag_start_y {
            let delta = start_y - current_y;
            let new_height = (self.state.queue.drag_start_height + delta).clamp(80.0, 480.0);
            self.state.queue.height = new_height;
            cx.notify();
        }
    }

    pub(crate) fn stop_resize_queue(&mut self, cx: &mut Context<Self>) {
        if self.state.queue.drag_start_y.is_some() {
            self.state.queue.drag_start_y = None;
            cx.notify();
        }
    }
}

impl AppShell {
    pub(crate) fn request_sftp_list_pane(
        &mut self,
        tab_id: TabId,
        target: SftpRemotePaneRef,
        path: String,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = self.sftp_view(tab_id) {
            view.update(cx, |view, cx| {
                view.request_list_pane(tab_id, target, path, cx)
            });
        }
    }

    pub(crate) fn request_sftp_list(
        &mut self,
        tab_id: TabId,
        path: String,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = self.sftp_view(tab_id) {
            view.update(cx, |view, cx| view.request_list(tab_id, path, cx));
        }
    }

    pub(crate) fn sftp_sync_with_terminal_cwd(
        &mut self,
        tab_id: TabId,
        cwd: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = self.sftp_view(tab_id) {
            view.update(cx, |view, cx| {
                view.sync_with_terminal_cwd(tab_id, cwd, window, cx)
            });
        }
    }

    pub(crate) fn sftp_start_resize_queue(
        &mut self,
        tab_id: TabId,
        start_y: f32,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = self.sftp_view(tab_id) {
            view.update(cx, |view, cx| view.start_resize_queue(start_y, cx));
        }
    }
}
