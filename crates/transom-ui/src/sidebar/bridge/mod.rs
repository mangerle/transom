//! 会话侧栏与外壳之间的事件路由及目录操作桥接。
//!
//! 设计原理（Why）：
//! - `SessionSidebarView` 与全部行渲染、表单定义都已脱离 `AppShell`；
//! - 本模块承接侧栏意图事件（连接、目录增删改、拖放排序、批量选择、拨测）
//!   以及拖放持久化，文件夹表单的打开/保存/渲染拆至 [`folder_form`]。

mod folder_form;

pub(crate) use folder_form::{open_new_folder, render_folder_form_body, save_folder_form};

use super::entity::SessionSidebarEvent;
use super::tree::{SidebarDrag, SidebarTarget};
use crate::app::{AppShell, app_settings};
use folder_form::open_rename_folder;
use gpui_kit::*;
use transom_core::{CatalogError, FolderId};

/// 处理会话侧栏发往外壳的意图事件。
pub(crate) fn on_session_sidebar_event(
    shell: &mut AppShell,
    event: &SessionSidebarEvent,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    match event {
        SessionSidebarEvent::ToggleSelectionMode => {
            crate::batch::toggle_selection_mode(shell, cx);
        }
        SessionSidebarEvent::SelectAllSessions => {
            crate::batch::select_all_sessions(shell, cx);
        }
        SessionSidebarEvent::OpenBatchEdit => {
            crate::batch::open_batch_edit(shell, window, cx);
        }
        SessionSidebarEvent::OpenNewFolder(parent) => {
            open_new_folder(shell, *parent, window, cx);
        }
        SessionSidebarEvent::OpenRenameFolder(folder_id) => {
            open_rename_folder(shell, *folder_id, window, cx);
        }
        SessionSidebarEvent::DeleteFolder(folder_id) => {
            delete_folder(shell, *folder_id, cx);
        }
        SessionSidebarEvent::ToggleFolderCollapsed(folder_id) => {
            toggle_folder_collapsed(shell, *folder_id, cx);
        }
        SessionSidebarEvent::OpenNewSession(None) => {
            shell.open_new_session(window, cx);
        }
        SessionSidebarEvent::OpenNewSession(Some(folder_id)) => {
            open_new_session_in_folder(shell, *folder_id, window, cx);
        }
        SessionSidebarEvent::ProbeFolder(folder_id) => {
            crate::probe::probe_folder(shell, *folder_id, cx);
        }
        SessionSidebarEvent::ProbeSession(session_id) => {
            crate::probe::probe_session(shell, *session_id, cx);
        }
        SessionSidebarEvent::Drop(drag, target) => {
            handle_sidebar_drop(shell, *drag, *target, cx);
        }
        SessionSidebarEvent::ToggleSessionSelection {
            session_id,
            selected,
        } => {
            crate::batch::toggle_session_selection(shell, *session_id, *selected, cx);
        }
        SessionSidebarEvent::SelectTreeSession(session_id) => {
            shell.state.session.sidebar.tree_selection = Some(*session_id);
            cx.notify();
        }
        SessionSidebarEvent::ConnectSession(session_id) => {
            crate::connection::connect_session(&mut shell.state, *session_id, window, cx);
            shell.reveal_vertical_tabs_panel(cx);
        }
        SessionSidebarEvent::OpenContainerWorkspace(session_id) => {
            crate::container::open_container_workspace_for_session(
                &mut shell.state,
                *session_id,
                window,
                cx,
            );
        }
        SessionSidebarEvent::DuplicateSession(session_id) => {
            shell.duplicate_session(*session_id, cx);
        }
        SessionSidebarEvent::EditSession(session_id) => {
            shell.open_edit_session(*session_id, window, cx);
        }
        SessionSidebarEvent::OpenAuditViewer(session_id) => {
            crate::audit_viewer::open_audit_viewer_for_session(shell, *session_id, window, cx);
        }
        SessionSidebarEvent::CopyHost(host) => {
            shell.state.ui.status =
                Some(rust_i18n::t!("sidebar.host_copied", host = host).to_string());
            cx.notify();
        }
        SessionSidebarEvent::DeleteSession(session_id) => {
            shell.delete_session_by_id(*session_id, cx);
        }
    }
}

/// 折叠或展开文件夹，并持久化收纳状态。
fn toggle_folder_collapsed(shell: &mut AppShell, id: FolderId, cx: &mut Context<AppShell>) {
    let collapsed = shell
        .state
        .session
        .catalog
        .folder(id)
        .map(|folder| !folder.collapsed);
    if let Some(collapsed) = collapsed {
        if let Err(error) = shell
            .state
            .session
            .catalog
            .set_folder_collapsed(id, collapsed)
        {
            shell.state.set_error(error.to_string());
            cx.notify();
            return;
        }
        shell.state.persist_catalog();
    }
    cx.notify();
}

/// 删除文件夹但保留全部内容，子项提升到父层级。
fn delete_folder(shell: &mut AppShell, folder_id: FolderId, cx: &mut Context<AppShell>) {
    match shell.state.session.catalog.delete_folder(folder_id) {
        Ok(name) => {
            shell.state.persist_catalog();
            shell.state.ui.status = Some(rust_i18n::t!("folder.deleted", name = name).to_string());
        }
        Err(error) => shell.state.set_error(error.to_string()),
    }
    cx.notify();
}

/// 在指定文件夹下新建会话，表单预填该文件夹路径。
fn open_new_session_in_folder(
    shell: &mut AppShell,
    folder_id: FolderId,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    let path = shell.state.session.catalog.folder_path(folder_id);
    let defaults = shell
        .state
        .session
        .catalog
        .inherited_folder_defaults(folder_id);
    let init = crate::session_form::SessionFormInit {
        base: None,
        folder_path: path.clone(),
        identities: shell.state.identity_options(),
        jump_options: shell.state.jump_options(None),
        folder_options: shell.state.folder_paths(),
        global_proxy: app_settings(cx).proxy.clone(),
        block_view: transom_core::BlockViewOverride::FollowGlobal,
    };
    let view = shell.state.ui.modals.session.form.clone();
    view.update(cx, |view, cx| {
        view.open(init, window, cx);
        if let Some(folder_path) = path {
            view.apply_folder_selection(&folder_path, &defaults, window, cx);
        }
    });
    cx.notify();
}

/// 处理会话/文件夹拖放：换父级或调整同级顺序。
fn handle_sidebar_drop(
    shell: &mut AppShell,
    drag: SidebarDrag,
    target: SidebarTarget,
    cx: &mut Context<AppShell>,
) {
    match apply_sidebar_drop(shell, drag, target) {
        Ok(landed) => {
            if let Some(folder_id) = landed
                && let Some(folder) = shell
                    .state
                    .session
                    .catalog
                    .folders
                    .iter_mut()
                    .find(|f| f.id == folder_id)
            {
                folder.collapsed = false;
            }
            shell.state.persist_catalog();
        }
        Err(error) => shell.state.set_error(error.to_string()),
    }
    cx.notify();
}

fn apply_sidebar_drop(
    shell: &mut AppShell,
    drag: SidebarDrag,
    target: SidebarTarget,
) -> Result<Option<FolderId>, CatalogError> {
    match (drag, target) {
        (SidebarDrag::Session(session_id), SidebarTarget::Root) => {
            shell.state.session.catalog.move_session(session_id, None)?;
            Ok(None)
        }
        (SidebarDrag::Folder(folder_id), SidebarTarget::Root) => {
            shell.state.session.catalog.move_folder(folder_id, None)?;
            Ok(None)
        }
        (SidebarDrag::Session(session_id), SidebarTarget::Folder(folder_id)) => {
            shell
                .state
                .session
                .catalog
                .move_session(session_id, Some(folder_id))?;
            Ok(Some(folder_id))
        }
        (SidebarDrag::Folder(moved_id), SidebarTarget::Folder(parent_id)) => {
            if moved_id == parent_id {
                return Ok(Some(parent_id));
            }
            shell
                .state
                .session
                .catalog
                .move_folder(moved_id, Some(parent_id))?;
            Ok(Some(parent_id))
        }
        (drag, SidebarTarget::Relative { anchor, side }) => match (drag, anchor) {
            (SidebarDrag::Session(session_id), SidebarDrag::Session(anchor_id)) => {
                shell
                    .state
                    .session
                    .catalog
                    .reorder_session(session_id, anchor_id, side)?;
                Ok(shell
                    .state
                    .session
                    .catalog
                    .folder_of_session(anchor_id)
                    .map(|folder| folder.id))
            }
            (SidebarDrag::Folder(folder_id), SidebarDrag::Folder(anchor_id)) => {
                shell
                    .state
                    .session
                    .catalog
                    .reorder_folder(folder_id, anchor_id, side)?;
                Ok(shell
                    .state
                    .session
                    .catalog
                    .folder(folder_id)
                    .and_then(|folder| folder.parent))
            }
            (SidebarDrag::Session(session_id), SidebarDrag::Folder(anchor_id)) => {
                shell
                    .state
                    .session
                    .catalog
                    .move_session(session_id, Some(anchor_id))?;
                Ok(Some(anchor_id))
            }
            (SidebarDrag::Folder(folder_id), SidebarDrag::Session(anchor_id)) => {
                let parent = shell
                    .state
                    .session
                    .catalog
                    .folder_of_session(anchor_id)
                    .map(|folder| folder.id);
                shell.state.session.catalog.move_folder(folder_id, parent)?;
                Ok(parent)
            }
        },
    }
}
