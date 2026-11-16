//! SFTP 列表、进度、校验、元数据与预览事件的处理与路由。

use crate::app::{AppShell, AppState};
use crate::sftp::{SftpPreviewResponse, remote_file_properties};
use gpui_kit::*;
use transom_core::SftpAppEvent;

/// SFTP 目录列表回填：委托目标窗格的面板实体按连接归属落位。
pub(crate) fn on_sftp_listing(
    app_state: &mut AppState,
    event: SftpAppEvent,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) -> bool {
    let SftpAppEvent::SftpListing {
        tab_id,
        connection_id,
        path,
        entries,
    } = event
    else {
        return true;
    };
    if let Some(view) = crate::sftp::sftp_view_entity_for_connection(
        &app_state.workspace,
        tab_id,
        connection_id,
        cx,
    ) {
        view.update(cx, |panel, cx| {
            panel.apply_listing(connection_id, path, entries, window, cx);
        });
    }
    true
}

/// SFTP 进度：高频可覆盖样本，只更新数据；仅可见标签自行触发重绘。
///
/// 返回 `false`：重绘由本方法条件决定，调用方不再统一 notify。
pub(crate) fn on_sftp_progress(
    app_state: &mut AppState,
    event: SftpAppEvent,
    cx: &mut Context<AppShell>,
) -> bool {
    let SftpAppEvent::SftpProgress {
        tab_id,
        connection_id,
        op_id,
        transferred,
        total,
        speed_bps,
        eta_secs,
    } = event
    else {
        return true;
    };
    let mut applied = false;
    if let Some(view) = crate::sftp::sftp_view_entity_for_connection(
        &app_state.workspace,
        tab_id,
        connection_id,
        cx,
    ) {
        view.update(cx, |panel, cx| {
            applied = panel.apply_progress(op_id, transferred, total, speed_bps, eta_secs, cx);
        });
    }
    if applied {
        if let Some(view) = app_state.sftp_view(tab_id) {
            view.update(cx, |view, _| view.note_drop_overlay_progress());
        }
        if app_state.is_rendered_pane(tab_id) {
            cx.notify();
        }
    }
    if applied && app_state.workspace.active_tab == Some(tab_id) && !app_state.ui.immersive {
        cx.notify();
    }
    false
}

/// SFTP 进入校验阶段：将任务状态更新为 `TransferStatus::Verifying`。
pub(crate) fn on_sftp_verifying(
    app_state: &mut AppState,
    event: SftpAppEvent,
    cx: &mut Context<AppShell>,
) -> bool {
    let SftpAppEvent::SftpVerifying {
        tab_id,
        connection_id,
        op_id,
    } = event
    else {
        return true;
    };
    if let Some(view) = crate::sftp::sftp_view_entity_for_connection(
        &app_state.workspace,
        tab_id,
        connection_id,
        cx,
    ) {
        view.update(cx, |panel, cx| panel.apply_verifying(op_id, cx));
    }
    true
}

/// SFTP 校验不一致后的自动恢复：任务回到传输中，进度归零。
pub(crate) fn on_sftp_retrying(
    app_state: &mut AppState,
    event: SftpAppEvent,
    cx: &mut Context<AppShell>,
) -> bool {
    let SftpAppEvent::SftpRetrying {
        tab_id,
        connection_id,
        op_id,
        reason,
    } = event
    else {
        return true;
    };
    if let Some(view) = crate::sftp::sftp_view_entity_for_connection(
        &app_state.workspace,
        tab_id,
        connection_id,
        cx,
    ) {
        view.update(cx, |panel, cx| panel.apply_retrying(op_id, &reason, cx));
    }
    true
}

/// SFTP 属性响应：迟到的响应（标签已关闭）直接忽略且不重绘。
pub(crate) fn on_sftp_metadata(
    app_state: &mut AppState,
    event: SftpAppEvent,
    cx: &mut Context<AppShell>,
) -> bool {
    let SftpAppEvent::SftpMetadata {
        tab_id,
        connection_id,
        metadata,
        ..
    } = event
    else {
        return true;
    };
    if crate::sftp::sftp_view_mut_for_connection(&app_state.workspace, tab_id, connection_id, cx)
        .is_none()
    {
        return false;
    }
    app_state.ui.modals.sftp.open_properties(
        rust_i18n::t!("sftp.modals.scope_remote").to_string(),
        remote_file_properties(&metadata),
        cx,
    );
    if let Some(view) = app_state.sftp_view(tab_id) {
        view.update(cx, |view, cx| view.process_queue(tab_id, cx));
    }
    true
}

/// SFTP 预览成功：交给预览响应处理统一落位。
pub(crate) fn on_sftp_preview(
    app_state: &mut AppState,
    event: SftpAppEvent,
    cx: &mut Context<AppShell>,
) -> bool {
    let SftpAppEvent::SftpPreview {
        tab_id,
        connection_id,
        op_id,
        bytes,
        truncated,
    } = event
    else {
        return true;
    };
    if crate::sftp::sftp_state_for_connection(&app_state.workspace, tab_id, connection_id, cx)
        .is_some()
        && let Some(view) = app_state.sftp_view(tab_id)
    {
        view.update(cx, |view, cx| {
            view.handle_preview(
                SftpPreviewResponse {
                    tab_id,
                    op_id,
                    result: Ok((bytes, truncated)),
                },
                cx,
            );
        });
    }
    true
}

/// SFTP 预览失败：与成功共用同一条响应处理路径。
pub(crate) fn on_sftp_preview_failed(
    app_state: &mut AppState,
    event: SftpAppEvent,
    cx: &mut Context<AppShell>,
) -> bool {
    let SftpAppEvent::SftpPreviewFailed {
        tab_id,
        connection_id,
        op_id,
        error,
    } = event
    else {
        return true;
    };
    if crate::sftp::sftp_state_for_connection(&app_state.workspace, tab_id, connection_id, cx)
        .is_some()
        && let Some(view) = app_state.sftp_view(tab_id)
    {
        view.update(cx, |view, cx| {
            view.handle_preview(
                SftpPreviewResponse {
                    tab_id,
                    op_id,
                    result: Err(error),
                },
                cx,
            );
        });
    }
    true
}

/// 目录对比完成：仅在同步模态仍匹配该操作时回填结果。
pub(crate) fn on_sftp_dir_compared(
    app_state: &mut AppState,
    event: SftpAppEvent,
    cx: &mut Context<AppShell>,
) -> bool {
    let SftpAppEvent::SftpDirCompared {
        connection_id,
        op_id,
        comparison,
        ..
    } = event
    else {
        return true;
    };
    if let Some(modal) = app_state.ui.modals.sftp.sync.clone() {
        modal.update(cx, |modal, cx| {
            modal.apply_dir_compared(connection_id, op_id, &comparison, cx);
        });
    }
    true
}
