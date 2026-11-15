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
            shell.state.sftp_modals.chmod = Some(crate::sftp::SftpChmodModalState {
                tab_id: *tab_id,
                connection_id: *connection_id,
                remote_path: remote_path.clone(),
                file_name: file_name.clone(),
                permissions: *permissions,
                octal_input: octal_input.clone(),
                error: None,
            });
            cx.notify();
        }
        SftpPanelEvent::OpenSearchModal(state) => {
            shell.state.sftp_modals.search = Some((**state).clone());
            cx.notify();
        }
        SftpPanelEvent::ShowProperties { scope, properties } => {
            shell.state.sftp_modals.properties = Some(crate::sftp::SftpPropertiesModalState {
                scope: scope.clone(),
                properties: properties.clone(),
            });
            cx.notify();
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
            shell.state.sftp_modals.confirm = Some(crate::sftp::SftpConfirmModalState {
                action: crate::sftp::SftpPendingDelete::Local {
                    tab_id: *tab_id,
                    paths: paths.clone(),
                },
                title: rust_i18n::t!("sftp.local_async.delete_title").to_string(),
                description: rust_i18n::t!("sftp.local_async.delete_body", count = paths.len())
                    .to_string(),
            });
            cx.notify();
        }
        SftpPanelEvent::ConfirmRemoteDelete {
            tab_id,
            connection_id,
            targets,
        } => {
            shell.state.sftp_modals.confirm = Some(crate::sftp::SftpConfirmModalState {
                action: crate::sftp::SftpPendingDelete::Remote {
                    tab_id: *tab_id,
                    connection_id: *connection_id,
                    targets: targets.clone(),
                },
                title: rust_i18n::t!("sftp.file_ops.delete_remote_title").to_string(),
                description: rust_i18n::t!(
                    "sftp.file_ops.delete_remote_body",
                    count = targets.len()
                )
                .to_string(),
            });
            cx.notify();
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
            shell.state.sftp_modals.sync = Some((**state).clone());
            cx.notify();
        }
        SftpPanelEvent::OpenFilterModal { textarea } => {
            shell.state.sftp_modals.filter = Some(crate::sftp::SftpFilterModalState {
                textarea: textarea.clone(),
                error: None,
            });
            cx.notify();
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
        // 需要弹窗状态的分支已由 try_open_sftp_modal 处理。
        _ => {}
    }
}
