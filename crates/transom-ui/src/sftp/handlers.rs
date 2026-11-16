//! SFTP 事件的域处理：完成/失败落定、桌面通知与面板尺寸联动。
//!
//! 与 [`crate::event_handlers`] 的分工：本文件只处理 SFTP 传输相关事件，
//! 连接/认证与遥测事件在另一处；拆分让每个文件保持在工程规范的体量内。

use crate::app::{AppShell, AppState, app_settings};
use crate::notify::send_desktop_notification;
use crate::palette::localized_error;
use crate::sftp::columns::SftpWidthAdjustment;
use crate::sftp::format_size;
use gpui_kit::*;
use transom_core::{AppSettings, SftpAppEvent, TransferDirection};

/// SFTP 传输完成：落定任务状态、刷新受影响的目录并派发桌面通知。
pub(crate) fn on_sftp_finished(
    app_state: &mut AppState,
    event: SftpAppEvent,
    cx: &mut Context<AppShell>,
) -> bool {
    let SftpAppEvent::SftpFinished {
        tab_id,
        connection_id,
        op_id,
        transferred,
        skipped,
        verified,
    } = event
    else {
        return true;
    };
    let Some(view) = crate::sftp::sftp_view_entity_for_connection(
        &app_state.workspace,
        tab_id,
        connection_id,
        cx,
    ) else {
        return true;
    };
    let outcome = view.update(cx, |panel, cx| {
        panel.settle_transfer_finished(connection_id, op_id, transferred, skipped, verified, cx)
    });
    view.update(cx, |view, cx| view.sync_drop_overlay(cx));
    if let Some((dir, file_name, bytes)) = outcome.notify_info {
        let action = match dir {
            TransferDirection::Upload => rust_i18n::t!("sftp.handlers.notify_upload_done"),
            TransferDirection::Download => rust_i18n::t!("sftp.handlers.notify_download_done"),
            TransferDirection::Sync => rust_i18n::t!("sftp.handlers.notify_sync_done"),
            TransferDirection::RemoteCopy => {
                rust_i18n::t!("sftp.handlers.notify_remote_copy_done")
            }
        };
        send_desktop_notification(
            &format!("Transom SFTP {action}"),
            &format!("{file_name} ({})", format_size(bytes)),
        );
    }
    if outcome.refresh_local {
        view.update(cx, |view, cx| view.local_refresh(cx));
    }
    if let Some(path) = outcome.refresh {
        crate::sftp::request_sftp_list(app_state, tab_id, path, cx);
    }
    if let Some((target_conn, target_dir)) = outcome.refresh_side {
        crate::sftp::request_sftp_list_for_connection(
            app_state,
            tab_id,
            target_conn,
            target_dir,
            cx,
        );
    }
    view.update(cx, |view, cx| view.process_queue(tab_id, cx));
    true
}

/// SFTP 失败：同步模态失败自行重绘并返回；传输失败落定任务并通知。
pub(crate) fn on_sftp_failed(
    app_state: &mut AppState,
    event: SftpAppEvent,
    cx: &mut Context<AppShell>,
) -> bool {
    let SftpAppEvent::SftpFailed {
        tab_id,
        connection_id,
        op_id,
        error,
    } = event
    else {
        return true;
    };
    if let Some(modal) = app_state.ui.modals.sftp.sync.clone() {
        let matched = modal.update(cx, |modal, cx| {
            if modal.state.op_id == op_id && modal.state.connection_id == connection_id {
                modal.state.scanning = false;
                modal.state.error = Some(localized_error(&error));
                cx.notify();
                true
            } else {
                false
            }
        });
        if matched {
            return false;
        }
    }
    let Some(view) = crate::sftp::sftp_view_entity_for_connection(
        &app_state.workspace,
        tab_id,
        connection_id,
        cx,
    ) else {
        return true;
    };
    let outcome = view.update(cx, |panel, cx| {
        panel.settle_transfer_failed(connection_id, op_id, &error, cx)
    });
    view.update(cx, |view, cx| view.sync_drop_overlay(cx));
    if let Some((file_name, err_msg)) = outcome.failed_notify {
        send_desktop_notification(
            &rust_i18n::t!("sftp.handlers.notify_failed_title"),
            &format!("{file_name}: {err_msg}"),
        );
    }
    view.update(cx, |view, cx| view.process_queue(tab_id, cx));
    true
}

/// 处理 SFTP 面板在列折叠/展开时产生的尺寸调整请求。
pub(crate) fn sync_sftp_panel_width(
    app_state: &mut AppState,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    let Some(tab_id) = app_state.workspace.active_tab else {
        return;
    };
    let adjustment = crate::sftp::sftp_view_mut(&app_state.workspace, tab_id, cx)
        .and_then(|mut panel| panel.state.pending_width_adjustment.take());
    let Some(adjustment) = adjustment else {
        return;
    };
    let Some(panel) = crate::sftp::sftp_state(&app_state.workspace, tab_id, cx) else {
        return;
    };
    if !panel.open {
        return;
    }
    let Some(index) = sftp_resizable_index(app_state, app_settings(cx)) else {
        return;
    };
    let panels = if app_settings(cx).window.sftp_panel_side.is_left() {
        app_state.ui.workspace_panels_left.clone()
    } else {
        app_state.ui.workspace_panels.clone()
    };
    let current_width = panels
        .read(cx)
        .sizes()
        .get(index)
        .copied()
        .map(|width| width.as_f32())
        .unwrap_or(360.0);

    let target = match adjustment {
        SftpWidthAdjustment::Exact(w) => w,
        SftpWidthAdjustment::Delta(d) => current_width + d,
    };
    let min_w = panel.min_content_width();
    let target = target.max(min_w);

    panels.update(cx, |state, cx| {
        state.resize_panel(index, px(target), window, cx)
    });
}

/// 把 SFTP 面板加宽 `delta`：挂载新列时现有面板放不下，直接向工作区分栏申请空间。
pub(crate) fn grow_sftp_panel_width(
    app_state: &mut AppState,
    delta: f32,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    if delta <= 0.5 {
        return;
    }
    let Some(index) = sftp_resizable_index(app_state, app_settings(cx)) else {
        return;
    };
    let panels = if app_settings(cx).window.sftp_panel_side.is_left() {
        app_state.ui.workspace_panels_left.clone()
    } else {
        app_state.ui.workspace_panels.clone()
    };
    let Some(current) = panels
        .read(cx)
        .sizes()
        .get(index)
        .copied()
        .map(|width| width.as_f32())
    else {
        return;
    };
    panels.update(cx, |state, cx| {
        state.resize_panel(index, px(current + delta), window, cx)
    });
}

/// SFTP 面板在工作区分组里的面板索引。
fn sftp_resizable_index(app_state: &AppState, settings: &AppSettings) -> Option<usize> {
    if app_state.ui.immersive {
        return None;
    }
    Some(if settings.window.sftp_panel_side.is_left() {
        1
    } else {
        2
    })
}
