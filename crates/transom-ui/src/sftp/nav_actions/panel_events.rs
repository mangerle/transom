//! SFTP 面板意图事件分发（弹窗打开、持久化设置、窗口级动作转发）。

use crate::app::{AppShell, AppState};
use gpui_kit::*;

/// 处理面板发来的意图事件。
///
/// 只使用事件携带的数据，不回读面板：事件派发阶段面板仍处于租借中。
/// 打开类事件：直接写入对应弹窗状态；返回是否已处理。
fn try_open_sftp_modal(
    app_state: &mut AppState,
    event: &crate::sftp::SftpPanelEvent,
    cx: &mut Context<AppShell>,
) -> bool {
    use crate::sftp::SftpPanelEvent;
    match event {
        SftpPanelEvent::OpenChmodModal {
            tab_id,
            connection_id,
            remote_path,
            file_name,
            permissions,
            octal_input,
        } => {
            app_state.ui.modals.sftp.open_chmod(
                crate::sftp::SftpChmodModalParams {
                    tab_id: *tab_id,
                    connection_id: *connection_id,
                    file_name: file_name.clone(),
                    remote_path: remote_path.clone(),
                    permissions: *permissions,
                    octal_input: octal_input.clone(),
                },
                cx,
            );
        }
        SftpPanelEvent::OpenSearchModal(state) => {
            app_state.ui.modals.sftp.open_search((**state).clone(), cx);
        }
        SftpPanelEvent::ShowProperties { scope, properties } => {
            app_state
                .ui
                .modals
                .sftp
                .open_properties(scope.clone(), properties.clone(), cx);
        }
        SftpPanelEvent::PersistSettings { status } => {
            let snapshot = crate::app::app_settings(cx).clone();
            if let Err(error) = app_state.persist_settings(&snapshot) {
                app_state.set_error(crate::palette::localized_error(&error));
            }
            app_state.ui.status = Some(status.clone());
            cx.notify();
        }
        SftpPanelEvent::ConfirmLocalDelete { tab_id, paths } => {
            app_state.ui.modals.sftp.open_confirm(
                crate::sftp::SftpPendingDelete::Local {
                    tab_id: *tab_id,
                    paths: paths.clone(),
                },
                rust_i18n::t!("sftp.local_async.delete_title").to_string(),
                rust_i18n::t!("sftp.local_async.delete_body", count = paths.len()).to_string(),
                cx,
            );
        }
        SftpPanelEvent::ConfirmRemoteDelete {
            tab_id,
            connection_id,
            targets,
        } => {
            app_state.ui.modals.sftp.open_confirm(
                crate::sftp::SftpPendingDelete::Remote {
                    tab_id: *tab_id,
                    connection_id: *connection_id,
                    targets: targets.clone(),
                },
                rust_i18n::t!("sftp.file_ops.delete_remote_title").to_string(),
                rust_i18n::t!("sftp.file_ops.delete_remote_body", count = targets.len())
                    .to_string(),
                cx,
            );
        }
        _ => return false,
    }
    true
}

pub(super) fn on_sftp_panel_event(
    app_state: &mut AppState,
    event: &crate::sftp::SftpPanelEvent,
    cx: &mut Context<AppShell>,
) {
    use crate::sftp::SftpPanelEvent;
    if try_open_sftp_modal(app_state, event, cx) {
        return;
    }
    match event {
        SftpPanelEvent::SendCommand(command) => {
            app_state.send_command(command.clone());
            cx.notify();
        }
        SftpPanelEvent::OpenEditor(_)
        | SftpPanelEvent::OpenInputModal(_)
        | SftpPanelEvent::GrowPanelWidth(_)
        | SftpPanelEvent::ChangeTerminalDirectory { .. } => {
            // 需要窗口的动作推到渲染期执行：事件派发时没有 Window。
            app_state.ui.window_requests.push_sftp(event.clone());
            cx.notify();
        }
        SftpPanelEvent::OpenSyncModal(state) => {
            app_state.ui.modals.sftp.open_sync((**state).clone(), cx);
        }
        SftpPanelEvent::OpenFilterModal { textarea } => {
            app_state.ui.modals.sftp.open_filter(textarea.clone(), cx);
        }
        SftpPanelEvent::ClosePanel { tab_id } => {
            app_state.close_sftp_panel(*tab_id);
            cx.notify();
        }
        SftpPanelEvent::OpenLocalDiff { tab_id, target } => {
            crate::diff_viewer::DiffViewerOps::new(app_state).open_local_diff_with_target(
                *tab_id,
                target.clone(),
                cx,
            );
        }
        SftpPanelEvent::SetStatus(message) => {
            app_state.ui.status = Some(message.clone());
            cx.notify();
        }
        SftpPanelEvent::OverlayChanged => {
            cx.notify();
        }
        // 需要弹窗状态的分支已由 try_open_sftp_modal 处理。
        _ => {}
    }
}
