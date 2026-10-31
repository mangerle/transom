//! SFTP 本地文件目录导航与快速跳转。
//!
//! 设计原理（Why）：
//! - 实现初衷：集中处理 SFTP 本地栏的路径跳转、双栏同步导航及在系统文件管理器中打开等操作；
//! - 核心优势：纯粹的本地文件系统导航流，避免远端与本地混在一起。

use crate::app::AppShell;
use crate::sftp::entity::SftpPanelView;
use crate::sftp::parent_remote;
use gpui_kit::*;
use std::path::PathBuf;
use transom_core::TabId;

impl SftpPanelView {
    pub(crate) fn local_navigate_only(
        &mut self,
        tab_id: TabId,
        target: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_hover_preview(cx);
        self.state.local.path = target.clone();
        // 先清空旧目录内容给出即时反馈，实际条目由后台刷新回填
        self.state.local.entries.clear();
        self.state.clear_local_selection();
        self.state.clear_local_error();
        let input = self.state.local.path_input.clone();
        input.update(cx, |state, cx| {
            state.set_value(target.display().to_string(), window, cx);
        });
        cx.notify();
        self.spawn_local_dir_refresh(None, cx);
        let _ = tab_id;
    }

    /// 终端目录静默跟随专用导航：更新路径与输入框，保留现有条目平滑过渡到
    /// 新目录读取完成，避免清空条目引起闪烁。
    pub(crate) fn local_sync_navigate(
        &mut self,
        tab_id: TabId,
        target: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_hover_preview(cx);
        self.state.local.path = target.clone();
        self.state.clear_local_selection();
        self.state.clear_local_error();
        let input = self.state.local.path_input.clone();
        input.update(cx, |state, cx| {
            state.set_value(target.display().to_string(), window, cx);
        });
        cx.notify();
        self.spawn_local_dir_refresh(None, cx);
        let _ = tab_id;
    }

    pub(crate) fn local_navigate(
        &mut self,
        tab_id: TabId,
        target: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let sync_remote_target = if self.state.sync_navigation {
            target
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| {
                    self.state
                        .remote
                        .entries
                        .iter()
                        .find(|entry| entry.is_dir && entry.name == name)
                        .map(|entry| entry.path.clone())
                })
        } else {
            None
        };
        self.local_navigate_only(tab_id, target, window, cx);
        if let Some(remote_path) = sync_remote_target {
            self.request_list(tab_id, remote_path, cx);
        }
    }

    pub(crate) fn local_go_up(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let local_parent = self
            .state
            .local
            .path
            .parent()
            .map(|path| path.to_path_buf());
        let remote_parent = self
            .state
            .sync_navigation
            .then(|| parent_remote(&self.state.remote.path));
        if let Some(parent) = local_parent {
            self.local_navigate_only(tab_id, parent, window, cx);
            if let Some(remote_parent) = remote_parent {
                self.request_list(tab_id, remote_parent, cx);
            }
        }
    }

    pub(crate) fn local_refresh(&mut self, cx: &mut Context<Self>) {
        self.dismiss_hover_preview(cx);
        self.spawn_local_dir_refresh(None, cx);
    }

    pub(crate) fn local_enter_path(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = PathBuf::from(self.state.local.path_input.read(cx).value().trim());
        if target.is_dir() {
            self.local_navigate(tab_id, target, window, cx);
        }
    }

    pub(crate) fn local_open_file_manager(&mut self, cx: &mut Context<Self>) {
        let local_path = self.state.local.path.clone();
        crate::audit::reveal_in_file_manager(local_path.clone());
        self.state.set_status(
            rust_i18n::t!(
                "sftp.nav.revealed_local_path",
                path = local_path.display().to_string().as_str()
            )
            .to_string(),
        );
        cx.notify();
    }

    /// 调用系统默认程序/关联编辑器打开指定的本地文件。
    pub(crate) fn local_edit_file(&mut self, target: PathBuf, cx: &mut Context<Self>) {
        if !target.is_file() {
            self.state
                .set_local_error(rust_i18n::t!("sftp.row.select_local_to_edit").to_string());
            cx.notify();
            return;
        }
        crate::audit::reveal_in_file_manager(target.clone());
        self.state.set_status(
            rust_i18n::t!(
                "sftp.nav.opened_with_default",
                path = target.display().to_string().as_str()
            )
            .to_string(),
        );
        cx.notify();
    }
}

impl AppShell {
    pub(crate) fn sftp_local_refresh(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        if let Some(view) = self.sftp_view(tab_id) {
            view.update(cx, |view, cx| view.local_refresh(cx));
        }
    }

    pub(crate) fn sftp_local_enter_path(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = self.sftp_view(tab_id) {
            view.update(cx, |view, cx| view.local_enter_path(tab_id, window, cx));
        }
    }
}
