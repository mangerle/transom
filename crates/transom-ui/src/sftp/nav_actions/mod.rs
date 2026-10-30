//! SFTP 远端目录导航、OSC 7 目录跟随与控制方法。
//!
//! 设计原理（Why）：
//! - 实现初衷：提供 SFTP 远端目录的路径跳转、向上级、回车、终端 cwd 联动与过滤切换；
//! - 核心优势：集中封装向底层通信通道投递 SftpOperation 的接口，保障状态同步一致性。

mod init;
mod local_nav;

use super::{SftpRemotePaneRef, normalize_local_cwd, normalize_remote_cwd};
use crate::app::AppShell;
use gpui_kit::*;
use std::path::Path;
use transom_core::{SftpCommand, SftpOperation, TabId, TransferId};

impl AppShell {
    /// 向目标远端列发起目录列举：主远端与并列远端共用同一套加载态与错误清理。
    pub(crate) fn request_sftp_list_pane(
        &mut self,
        tab_id: TabId,
        target: SftpRemotePaneRef,
        path: String,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_sftp_hover_preview(tab_id, cx);
        let Some(connection_id) = self.remote_pane_connection(tab_id, target) else {
            log::warn!("远端目录请求找不到目标列，窗格: {tab_id}，目标: {target:?}，路径: {path}");
            return;
        };
        log::debug!(
            "请求远端目录列表，窗格: {tab_id}，目标: {target:?}，连接: {connection_id}，路径: {path}"
        );
        self.begin_sftp_pane_load(tab_id, target);
        self.send_command(SftpCommand::Sftp {
            tab_id,
            connection_id,
            op_id: TransferId::new(),
            op: SftpOperation::List { path },
        });
        cx.notify();
    }

    pub(crate) fn request_sftp_list(
        &mut self,
        tab_id: TabId,
        path: String,
        cx: &mut Context<Self>,
    ) {
        self.request_sftp_list_pane(tab_id, SftpRemotePaneRef::Main, path, cx);
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
