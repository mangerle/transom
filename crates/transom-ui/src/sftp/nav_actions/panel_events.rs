//! SFTP 面板意图事件分发（弹窗打开、持久化设置、窗口级动作转发）。

use crate::app::AppShell;
use gpui_kit::*;

/// 处理面板发来的意图事件。
///
/// 只使用事件携带的数据，不回读面板：事件派发阶段面板仍处于租借中。
/// 打开类事件：直接写入对应弹窗状态；返回是否已处理。
fn try_open_sftp_modal(
    shell: &mut AppShell,
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
            crate::sftp::open_sftp_chmod_modal(
                shell,
                *tab_id,
                *connection_id,
                file_name.clone(),
                remote_path.clone(),
                *permissions,
                octal_input.clone(),
                cx,
            );
        }
        SftpPanelEvent::OpenSearchModal(state) => {
            crate::sftp::open_sftp_search_modal(shell, (**state).clone(), cx);
        }
        SftpPanelEvent::ShowProperties { scope, properties } => {
            crate::sftp::open_sftp_properties_modal(
                shell,
                scope.clone(),
                properties.clone(),
                cx,
            );
        }
        SftpPanelEvent::PersistSettings { status } => {
            let snapshot = crate::app::app_settings(cx).clone();
            if let Err(error) = shell.state.persist_settings(&snapshot) {
                shell
                    .state
                    .set_error(crate::palette::localized_error(&error));
            }
            shell.state.status = Some(status.clone());
            cx.notify();
        }
        SftpPanelEvent::ConfirmLocalDelete { tab_id, paths } => {
            crate::sftp::open_sftp_confirm_modal(
                shell,
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
            crate::sftp::open_sftp_confirm_modal(
                shell,
                crate::sftp::SftpPendingDelete::Remote {
                    tab_id: *tab_id,
                    connection_id: *connection_id,
                    targets: targets.clone(),
                },
                rust_i18n::t!("sftp.file_ops.delete_remote_title").to_string(),
                rust_i18n::t!("sftp.file_ops.delete_remote_body", count = targets.len()).to_string(),
                cx,
            );
        }
        _ => return false,
    }
    true
}

pub(super) fn on_sftp_panel_event(
    shell: &mut AppShell,
    event: &crate::sftp::SftpPanelEvent,
    cx: &mut Context<AppShell>,
) {
    use crate::sftp::SftpPanelEvent;
    if try_open_sftp_modal(shell, event, cx) {
        return;
    }
    match event {
        SftpPanelEvent::SendCommand(command) => {
            shell.state.send_command(command.clone());
            cx.notify();
        }
        SftpPanelEvent::OpenEditor(_)
        | SftpPanelEvent::OpenInputModal(_)
        | SftpPanelEvent::GrowPanelWidth(_) => {
            // 需要窗口的动作推到渲染期执行：事件派发时没有 Window。
            shell.state.window_requests.push_sftp(event.clone());
        }
        SftpPanelEvent::OpenSyncModal(state) => {
            crate::sftp::open_sftp_sync_modal(shell, (**state).clone(), cx);
        }
        SftpPanelEvent::OpenFilterModal { textarea } => {
            crate::sftp::open_sftp_filter_modal(shell, textarea.clone(), cx);
        }
        SftpPanelEvent::ClosePanel { tab_id } => {
            shell.state.close_sftp_panel(*tab_id);
            cx.notify();
        }
        SftpPanelEvent::OpenLocalDiff { tab_id, target } => {
            crate::diff_viewer::open_local_diff_with_target(shell, *tab_id, target.clone(), cx);
        }
        SftpPanelEvent::SetStatus(message) => {
            shell.state.status = Some(message.clone());
            cx.notify();
        }
        SftpPanelEvent::OverlayChanged => {
            cx.notify();
        }
        // 需要弹窗状态的分支已由 try_open_sftp_modal 处理。
        _ => {}
    }
}
