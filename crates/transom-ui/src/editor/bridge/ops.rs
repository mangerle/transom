//! 编辑器读写命令发送、提权探测与结果回写桥接。

use crate::app::{AppShell, AppState};
use crate::editor::open::parent_dir;
use crate::editor::ops::{
    EditorFailureAction, EditorWrittenAction, SudoPasswordRequest, SudoProbeOutcome,
    SudoProbeRequest,
};
use crate::sftp::{SftpInputModalParams, SftpInputTarget};
use gpui_kit::*;
use transom_core::{
    ConnectionId, ErrorCode, RemoteTextMeta, SftpCommand, SftpOperation, SudoCapability, TabId,
    TransferId,
};

/// 向远端发起提权能力探测；结果由 `on_sudo_probed` 回填。
fn send_sudo_probe(app_state: &mut AppState, request: &SudoProbeRequest) -> Result<(), ()> {
    if app_state.send_command(SftpCommand::Sftp {
        tab_id: request.tab_id,
        connection_id: request.connection_id,
        op_id: request.op_id,
        op: SftpOperation::ProbeSudo,
    }) {
        Ok(())
    } else {
        Err(())
    }
}

/// 打开 Sudo 口令输入框。
fn open_sudo_password_modal(
    app_state: &mut AppState,
    request: SudoPasswordRequest,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    app_state.ui.modals.sftp.open_input(
        SftpInputModalParams {
            tab_id: request.tab_id,
            connection_id: None,
            title: rust_i18n::t!("editor.sudo_save").to_string(),
            prompt: rust_i18n::t!("editor.sudo_save_prompt", file_name = request.file_name)
                .to_string(),
            placeholder: rust_i18n::t!("editor.sudo_password_hint").to_string(),
            value: String::new(),
            target: SftpInputTarget::SudoElevation {
                editor: request.editor,
            },
        },
        window,
        cx,
    );
}

/// 回传成功事件：先由实体落到新基线，再执行目录补丁、焦点与续行关闭。
pub(crate) fn on_editor_text_written(
    app_state: &mut AppState,
    op_id: TransferId,
    meta: RemoteTextMeta,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    let Some(action) = app_state.features.editors.update(cx, |panels, cx| {
        panels.note_editor_text_written(op_id, meta, cx)
    }) else {
        return;
    };
    apply_editor_written(app_state, action, window, cx);
}

fn apply_editor_written(
    app_state: &mut AppState,
    action: EditorWrittenAction,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    if let Some(mut panel) = crate::sftp::sftp_view_mut(&app_state.workspace, action.tab_id, cx)
        && let Some(entry) = panel
            .state
            .remote
            .entries
            .iter_mut()
            .find(|entry| entry.path == action.remote_path)
    {
        entry.size = action.meta.size;
        entry.modified = action.meta.mtime;
    }
    if action.was_new_file
        && let Some(parent) = parent_dir(&action.remote_path)
        && crate::sftp::sftp_state(&app_state.workspace, action.tab_id, cx)
            .is_some_and(|panel| panel.remote.path == parent)
    {
        crate::sftp::request_sftp_list(app_state, action.tab_id, parent.to_string(), cx);
    }
    if action.dropped {
        crate::sftp::focus_active_sftp_panel(app_state, window, cx);
    }
    if let Some(intent) = action.pending_close {
        if let Some(request) =
            super::try_finish_pending_close(app_state, intent, &action.dirty_tabs, cx)
        {
            app_state.ui.window_requests.push_editor_close(request);
        }
    } else {
        cx.notify();
    }
}

/// 回传冲突事件。
pub(crate) fn note_editor_text_conflict(
    app_state: &mut AppState,
    op_id: TransferId,
    message: String,
    cx: &mut Context<AppShell>,
) {
    app_state.features.editors.update(cx, |panels, cx| {
        panels.note_editor_text_conflict(op_id, message, cx)
    });
}

/// 读取或回传失败事件：按实体返回的决策投递探测或重新询问口令。
pub(crate) fn note_editor_text_failed(
    app_state: &mut AppState,
    op_id: TransferId,
    message: String,
    code: ErrorCode,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    let action = app_state.features.editors.update(cx, |panels, cx| {
        panels.note_editor_text_failed(op_id, message, code, cx)
    });
    match action {
        EditorFailureAction::Probe(request) => {
            dispatch_sudo_probe(app_state, request, cx);
        }
        EditorFailureAction::AskPassword(request) => {
            open_sudo_password_modal(app_state, request, window, cx);
        }
        EditorFailureAction::None => {}
    }
}

/// 提权探测结果事件：免密直接写、需要口令开弹窗、不可用已就地显示原因。
pub(crate) fn on_sudo_probed(
    app_state: &mut AppState,
    op_id: TransferId,
    capability: SudoCapability,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    let outcome = app_state.features.editors.update(cx, |panels, cx| {
        panels.note_sudo_probed(op_id, capability, cx)
    });
    match outcome {
        SudoProbeOutcome::WritePasswordless { editor } => {
            app_state.features.editors.update(cx, |panels, cx| {
                panels.submit_sudo_editor_write(&editor, None, cx);
            });
        }
        SudoProbeOutcome::AskPassword(request) => {
            open_sudo_password_modal(app_state, request, window, cx);
        }
        SudoProbeOutcome::None => {}
    }
}

/// 把编辑入口的拒绝原因写到目标远端列的错误位上。
pub(crate) fn editor_reject(
    app_state: &mut AppState,
    tab_id: TabId,
    connection_id: ConnectionId,
    message: String,
    cx: &mut Context<AppShell>,
) {
    if let Some(main_connection) = app_state
        .workspace
        .tab(tab_id)
        .and_then(|tab| tab.connection_id)
        && let Some(mut panel) = crate::sftp::sftp_view_mut(&app_state.workspace, tab_id, cx)
        && let Some(target) = panel
            .state
            .remote_pane_of_connection(connection_id, Some(main_connection))
        && let Some(pane) = panel.state.remote_pane_mut(target)
    {
        pane.error = Some(message);
    }
    cx.notify();
}

/// 投递提权探测；失败时把错误落回实体并交给面板显示。
fn dispatch_sudo_probe(
    app_state: &mut AppState,
    request: SudoProbeRequest,
    cx: &mut Context<AppShell>,
) {
    let op_id = request.op_id;
    if send_sudo_probe(app_state, &request).is_err() {
        app_state
            .features
            .editors
            .update(cx, |panels, cx| panels.fail_sudo_probe_dispatch(op_id, cx));
    }
}
