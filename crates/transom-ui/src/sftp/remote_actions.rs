//! 远端列的动作辅助入口：按目标列解析连接、上传与跨列复制。
//!
//! 设计原理（Why）：
//! - 实现初衷：并列远端列此前只实现了部分动作，主远端的导航、目录操作与
//!   上下文菜单各写一套；统一状态后，动作直接按目标列寻址，不再默认落到
//!   `panel.remote`；
//! - 核心优势：文件操作、弹窗与导航都只有一份连接化实现，调用方只传
//!   `SftpRemotePaneRef`，新增列能力不需要再补一套并列列分支；
//! - 代价与局限：主远端与本地栏共用弹窗入口时仍需区分"远端目标/本地目标"，
//!   这属于本地与远端两种语义，不再是主列与并列列的重复实现。
//!
//! 导航与过滤动作在 `remote_nav`，文件操作实现分散在各自的业务文件中。

use super::entity::{SftpPanelView, read_shell_parts, with_shell_parts};
use crate::app::AppShell;
use crate::app::app_settings;
use crate::sftp::side_remote::SftpRemoteCopyParams;
use crate::sftp::{SftpRemotePaneRef, SftpRemotePaneState, format_size, parent_remote};
use gpui_kit::*;
use transom_core::{ConnectionId, SftpCommand, SftpOperation, TabId, TransferId};

/// 一次远端列动作的操作域：目标列、实际连接与弹窗应携带的连接语义。
///
/// `side_connection` 为 `None` 表示主远端（弹窗提交时沿用标签页主连接），
/// `Some` 表示并列列。集中在这里解析，避免每个动作各写一遍查找口径。
pub(crate) struct RemotePaneScope<'a> {
    pub(crate) pane: &'a SftpRemotePaneState,
    pub(crate) connection_id: ConnectionId,
}

impl SftpPanelView {
    /// 向指定连接下发 SFTP 非传输操作。
    pub(crate) fn send_op_for_connection(
        &mut self,
        connection_id: ConnectionId,
        op: SftpOperation,
        cx: &mut Context<Self>,
    ) {
        let app = self.app.clone();
        let Some(main_connection) = read_shell_parts(&app, cx, |shell| {
            shell
                .state
                .workspace
                .tab(self.tab_id)
                .and_then(|tab| tab.connection_id)
        })
        .flatten() else {
            return;
        };
        let Some(target) = self
            .state
            .remote_pane_of_connection(connection_id, Some(main_connection))
        else {
            return;
        };
        self.state.clear_status();
        if let Some(pane) = self.state.remote_pane_mut(target) {
            pane.error = None;
        }
        let tab_id = self.tab_id;
        with_shell_parts(&app, cx, |shell, _| {
            shell.state.send_command(SftpCommand::Sftp {
                tab_id,
                connection_id,
                op_id: TransferId::new(),
                op,
            });
        });
        cx.notify();
    }

    /// 把目标远端列设为活动栏并重绘。
    pub(crate) fn activate_pane(&mut self, target: SftpRemotePaneRef, cx: &mut Context<Self>) {
        self.state.active_pane = target.into();
        cx.notify();
    }

    /// 右键菜单定位条目：已多选时只改主项，否则清空旧选择只选当前项。
    pub(crate) fn select_entry_for_menu(
        &mut self,
        target: SftpRemotePaneRef,
        path: String,
        cx: &mut Context<Self>,
    ) {
        if let Some(pane) = self.state.remote_pane_mut(target) {
            if pane.selection.contains(&path) {
                pane.set_primary(path);
            } else {
                pane.select(path);
            }
            self.state.active_pane = target.into();
        }
        cx.notify();
    }

    /// 在目标列当前目录直接解压（不弹命名框）。
    pub(crate) fn extract_here(
        &mut self,
        target: SftpRemotePaneRef,
        archive_path: String,
        cx: &mut Context<Self>,
    ) {
        let app = self.app.clone();
        let Some(main_connection) = read_shell_parts(&app, cx, |shell| {
            shell
                .state
                .workspace
                .tab(self.tab_id)
                .and_then(|tab| tab.connection_id)
        })
        .flatten() else {
            return;
        };
        let Some(connection_id) = self.state.remote_connection(target, Some(main_connection))
        else {
            return;
        };
        let dest_dir = parent_remote(&archive_path);
        self.send_op_for_connection(
            connection_id,
            SftpOperation::Extract {
                archive_path,
                dest_dir,
            },
            cx,
        );
    }

    /// 目标列的编辑器入口。
    pub(crate) fn edit_remote_file(
        &mut self,
        target: SftpRemotePaneRef,
        path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let app = self.app.clone();
        let Some((main_connection, max_bytes)) = read_shell_parts(&app, cx, |shell| {
            (
                shell
                    .state
                    .workspace
                    .tab(self.tab_id)
                    .and_then(|tab| tab.connection_id),
                app_settings(cx).sftp.edit_max_bytes,
            )
        }) else {
            return;
        };
        let Some(connection_id) = self.state.remote_connection(target, main_connection) else {
            return;
        };
        let Some(entry) = self.state.remote_pane(target).and_then(|pane| {
            pane.entries
                .iter()
                .find(|entry| entry.path == path)
                .cloned()
        }) else {
            if let Some(pane) = self.state.remote_pane_mut(target) {
                pane.error = Some(rust_i18n::t!("sftp.select_file").to_string());
            }
            cx.notify();
            return;
        };
        if entry.is_dir {
            return;
        }
        if entry.size > max_bytes {
            if let Some(pane) = self.state.remote_pane_mut(target) {
                pane.error = Some(
                    rust_i18n::t!(
                        "editor.error_too_large",
                        name = entry.name,
                        size = format_size(entry.size),
                        max = format_size(max_bytes)
                    )
                    .to_string(),
                );
            }
            cx.notify();
            return;
        }
        let tab_id = self.tab_id;
        with_shell_parts(&app, cx, |shell, cx| {
            shell.open_remote_editor_panel(
                crate::editor::RemoteEditorOpen {
                    tab_id,
                    connection_id,
                    remote_path: path,
                    file_name: entry.name,
                    permissions: entry.permissions,
                },
                window,
                cx,
            );
        });
    }

    /// 目标列的上传入口：把本地列选中项上传到该列当前目录。
    pub(crate) fn upload_local_selection(
        &mut self,
        target: SftpRemotePaneRef,
        cx: &mut Context<Self>,
    ) {
        let app = self.app.clone();
        let Some(main_connection) = read_shell_parts(&app, cx, |shell| {
            shell
                .state
                .workspace
                .tab(self.tab_id)
                .and_then(|tab| tab.connection_id)
        })
        .flatten() else {
            return;
        };
        let paths = self.state.selected_local_paths();
        let Some(connection_id) = self.state.remote_connection(target, Some(main_connection))
        else {
            return;
        };
        let Some(target_dir) = self.state.remote_pane(target).map(|pane| pane.path.clone()) else {
            return;
        };
        if paths.is_empty() {
            self.set_pane_error(target, rust_i18n::t!("sftp.select_file").to_string(), cx);
            return;
        }
        if let Err(message) =
            self.enqueue_uploads_to(self.tab_id, &paths, &target_dir, connection_id, cx)
        {
            self.set_pane_error(target, message, cx);
        }
    }

    /// 目标列的"复制到主远端"入口（并列列专属菜单项）。
    pub(crate) fn copy_entry_to_main(
        &mut self,
        source: SftpRemotePaneRef,
        path: String,
        is_dir: bool,
        cx: &mut Context<Self>,
    ) {
        let app = self.app.clone();
        let Some(main_connection) = read_shell_parts(&app, cx, |shell| {
            shell
                .state
                .workspace
                .tab(self.tab_id)
                .and_then(|tab| tab.connection_id)
        })
        .flatten() else {
            return;
        };
        let Some(source_connection_id) =
            self.state.remote_connection(source, Some(main_connection))
        else {
            return;
        };
        let target_dir = self.state.remote.path.clone();
        let fname = path.rsplit('/').next().unwrap_or(&path);
        let target_path = format!("{}/{}", target_dir.trim_end_matches('/'), fname);
        self.enqueue_remote_transfer(
            self.tab_id,
            SftpRemoteCopyParams {
                source_connection_id,
                source_path: path,
                target_connection_id: main_connection,
                target_path,
                is_dir,
            },
            cx,
        );
    }

    /// 把错误落到目标列的内容区。
    pub(crate) fn set_pane_error(
        &mut self,
        target: SftpRemotePaneRef,
        message: String,
        cx: &mut Context<Self>,
    ) {
        if let Some(pane) = self.state.remote_pane_mut(target) {
            pane.error = Some(message);
        }
        cx.notify();
    }
}

impl AppShell {
    pub(crate) fn remote_pane_scope<'a>(
        &self,
        tab_id: TabId,
        target: SftpRemotePaneRef,
        cx: &'a App,
    ) -> Option<RemotePaneScope<'a>> {
        let tab = self.state.workspace.tab(tab_id)?;
        let panel = self.sftp_state(tab_id, cx)?;
        let pane = panel.remote_pane(target)?;
        let connection_id = panel.remote_connection(target, tab.connection_id)?;
        Some(RemotePaneScope {
            pane,
            connection_id,
        })
    }

    pub(crate) fn send_sftp_op_for_connection(
        &mut self,
        tab_id: TabId,
        connection_id: ConnectionId,
        op: SftpOperation,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = self.sftp_view(tab_id) {
            view.update(cx, |view, cx| {
                view.send_op_for_connection(connection_id, op, cx)
            });
        }
    }
}
