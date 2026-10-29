//! SFTP 本地文件目录导航与快速跳转。
//!
//! 设计原理（Why）：
//! - 实现初衷：集中处理 SFTP 本地栏的路径跳转、双栏同步导航及在系统文件管理器中打开等操作；
//! - 核心优势：纯粹的本地文件系统导航流，避免远端与本地混在一起。

use crate::app::AppShell;
use crate::sftp::parent_remote;
use gpui_kit::*;
use std::path::PathBuf;
use transom_core::TabId;

impl AppShell {
    pub(crate) fn sftp_local_navigate_only(
        &mut self,
        tab_id: TabId,
        target: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_sftp_hover_preview(tab_id, cx);
        let Some(tab) = self.workspace.tab_mut(tab_id) else {
            return;
        };
        let Some(panel) = tab.sftp_mut() else {
            return;
        };
        panel.local.path = target.clone();
        // 先清空旧目录内容给出即时反馈，实际条目由后台刷新回填
        panel.local.entries = Vec::new();
        panel.clear_local_selection();
        panel.clear_local_error();
        let input = panel.local.path_input.clone();
        input.update(cx, |state, cx| {
            state.set_value(target.display().to_string(), window, cx);
        });
        cx.notify();
        self.spawn_local_dir_refresh(tab_id, None, cx);
    }

    /// 终端目录静默跟随专用导航：更新路径与输入框，保留现有条目平滑过渡至新目录读取完成，避免清空条目引起闪烁。
    pub(crate) fn sftp_local_sync_navigate(
        &mut self,
        tab_id: TabId,
        target: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_sftp_hover_preview(tab_id, cx);
        let Some(tab) = self.workspace.tab_mut(tab_id) else {
            return;
        };
        let Some(panel) = tab.sftp_mut() else {
            return;
        };
        panel.local.path = target.clone();
        panel.clear_local_selection();
        panel.clear_local_error();
        let input = panel.local.path_input.clone();
        input.update(cx, |state, cx| {
            state.set_value(target.display().to_string(), window, cx);
        });
        cx.notify();
        self.spawn_local_dir_refresh(tab_id, None, cx);
    }

    pub(crate) fn sftp_local_navigate(
        &mut self,
        tab_id: TabId,
        target: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let sync_remote_target = self
            .workspace
            .tab(tab_id)
            .and_then(|tab| tab.sftp())
            .and_then(|panel| {
                if !panel.sync_navigation {
                    return None;
                }
                let child_name = target.file_name()?.to_str()?;
                panel
                    .remote
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
            .workspace
            .tab(tab_id)
            .and_then(|tab| tab.sftp())
            .map(|panel| {
                let local_parent = panel.local.path.parent().map(|p| p.to_path_buf());
                let remote_parent = if panel.sync_navigation {
                    Some(parent_remote(&panel.remote.path))
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
        self.dismiss_sftp_hover_preview(tab_id, cx);
        self.spawn_local_dir_refresh(tab_id, None, cx);
    }

    pub(crate) fn sftp_local_enter_path(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = self
            .workspace
            .tab(tab_id)
            .and_then(|tab| tab.sftp())
            .map(|panel| PathBuf::from(panel.local.path_input.read(cx).value().trim()));
        if let Some(target) = target
            && target.is_dir()
        {
            self.sftp_local_navigate(tab_id, target, window, cx);
        }
    }

    pub(crate) fn sftp_open_local_file_manager(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(local_path) = self
            .workspace
            .tab(tab_id)
            .and_then(|tab| tab.sftp())
            .map(|panel| panel.local.path.clone())
        else {
            return;
        };
        crate::audit::reveal_in_file_manager(local_path.clone());

        if let Some(panel) = self
            .workspace
            .tab_mut(tab_id)
            .and_then(|tab| tab.sftp_mut())
        {
            panel.set_status(
                rust_i18n::t!(
                    "sftp.nav.revealed_local_path",
                    path = local_path.display().to_string().as_str()
                )
                .to_string(),
            );
            cx.notify();
        }
    }

    /// 调用系统默认程序/关联编辑器打开指定的本地文件。
    pub(crate) fn sftp_edit_local_file(
        &mut self,
        tab_id: TabId,
        target: PathBuf,
        cx: &mut Context<Self>,
    ) {
        if !target.is_file() {
            if let Some(panel) = self.workspace.tab_mut(tab_id).and_then(|t| t.sftp_mut()) {
                panel.set_local_error(rust_i18n::t!("sftp.row.select_local_to_edit").to_string());
                cx.notify();
            }
            return;
        }
        crate::audit::reveal_in_file_manager(target.clone());
        if let Some(panel) = self.workspace.tab_mut(tab_id).and_then(|t| t.sftp_mut()) {
            panel.set_status(
                rust_i18n::t!(
                    "sftp.nav.opened_with_default",
                    path = target.display().to_string().as_str()
                )
                .to_string(),
            );
            cx.notify();
        }
    }
}
