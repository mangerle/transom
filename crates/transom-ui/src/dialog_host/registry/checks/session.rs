//! 会话、连接、工具、SFTP、编辑器与差异视图的弹窗判定。
use super::super::DialogCheckState;
use gpui_kit::App;

pub(in crate::dialog_host) fn session_form_open(state: &DialogCheckState, cx: &App) -> bool {
    state.modals.session.form.read(cx).is_open()
}

pub(in crate::dialog_host) fn session_form_values(
    state: &DialogCheckState,
    cx: &App,
) -> Vec<String> {
    state.modals.session.form.read(cx).text_values(cx)
}

pub(in crate::dialog_host) fn folder_form_open(state: &DialogCheckState, _: &App) -> bool {
    state.modals.session.folder_form.is_some()
}

pub(in crate::dialog_host) fn folder_form_values(
    state: &DialogCheckState,
    cx: &App,
) -> Vec<String> {
    state
        .modals
        .session
        .folder_form
        .as_ref()
        .map(|form| form.text_values(cx))
        .unwrap_or_default()
}

pub(in crate::dialog_host) fn batch_edit_open(state: &DialogCheckState, _: &App) -> bool {
    state.modals.session.batch_edit.is_some()
}

pub(in crate::dialog_host) fn batch_edit_values(state: &DialogCheckState, cx: &App) -> Vec<String> {
    state
        .modals
        .session
        .batch_edit
        .as_ref()
        .map(|form| form.text_values(cx))
        .unwrap_or_default()
}

pub(in crate::dialog_host) fn auth_prompt_open(state: &DialogCheckState, cx: &App) -> bool {
    state.modals.connection.prompts.read(cx).is_auth_open()
}

pub(in crate::dialog_host) fn host_key_confirm_open(state: &DialogCheckState, cx: &App) -> bool {
    state.modals.connection.prompts.read(cx).is_host_key_open()
}

pub(in crate::dialog_host) fn clipboard_prompt_open(state: &DialogCheckState, cx: &App) -> bool {
    state.modals.connection.prompts.read(cx).is_clipboard_open()
}

pub(in crate::dialog_host) fn known_hosts_open(state: &DialogCheckState, cx: &App) -> bool {
    state.modals.connection.known_hosts.read(cx).is_open()
}

pub(in crate::dialog_host) fn forward_confirm_open(state: &DialogCheckState, _: &App) -> bool {
    state.modals.tools.confirm_forward_any.is_some()
}

pub(in crate::dialog_host) fn key_wizard_open(state: &DialogCheckState, cx: &App) -> bool {
    state.modals.tools.key_wizard.read(cx).is_open()
}

pub(in crate::dialog_host) fn backup_prompt_open(state: &DialogCheckState, cx: &App) -> bool {
    state.modals.tools.backup_prompt.read(cx).is_open()
}

pub(in crate::dialog_host) fn audit_viewer_open(state: &DialogCheckState, cx: &App) -> bool {
    state.modals.tools.audit_viewer.read(cx).is_open()
}

pub(in crate::dialog_host) fn replay_open(state: &DialogCheckState, cx: &App) -> bool {
    state.modals.tools.replay.read(cx).is_open()
}

pub(in crate::dialog_host) fn snippet_form_open(state: &DialogCheckState, _: &App) -> bool {
    state.modals.tools.snippet_panel.form.is_some()
}

pub(in crate::dialog_host) fn snippet_form_values(
    state: &DialogCheckState,
    cx: &App,
) -> Vec<String> {
    state
        .modals
        .tools
        .snippet_panel
        .form
        .as_ref()
        .map(|form| form.text_values(cx))
        .unwrap_or_default()
}

pub(in crate::dialog_host) fn settings_open(state: &DialogCheckState, cx: &App) -> bool {
    state.modals.settings.read(cx).is_open()
}

pub(in crate::dialog_host) fn shortcut_capture_open(state: &DialogCheckState, _: &App) -> bool {
    state.shortcut_recording.capture.is_some()
}

// ---- SFTP ----

pub(in crate::dialog_host) fn sftp_confirm_open(state: &DialogCheckState, _: &App) -> bool {
    state.modals.sftp.confirm.is_some()
}

pub(in crate::dialog_host) fn sftp_input_open(state: &DialogCheckState, _: &App) -> bool {
    state.modals.sftp.input.is_some()
}

pub(in crate::dialog_host) fn sftp_chmod_open(state: &DialogCheckState, _: &App) -> bool {
    state.modals.sftp.chmod.is_some()
}

pub(in crate::dialog_host) fn sftp_properties_open(state: &DialogCheckState, _: &App) -> bool {
    state.modals.sftp.properties.is_some()
}

pub(in crate::dialog_host) fn sftp_filter_open(state: &DialogCheckState, _: &App) -> bool {
    state.modals.sftp.filter.is_some()
}

pub(in crate::dialog_host) fn sftp_search_open(state: &DialogCheckState, _: &App) -> bool {
    state.modals.sftp.search.is_some()
}

pub(in crate::dialog_host) fn sftp_sync_open(state: &DialogCheckState, _: &App) -> bool {
    state.modals.sftp.sync.is_some()
}

pub(in crate::dialog_host) fn editor_goto_line_open(state: &DialogCheckState, _: &App) -> bool {
    state.modals.tools.editor_goto_line.is_some()
}

pub(in crate::dialog_host) fn diff_viewer_open(state: &DialogCheckState, _: &App) -> bool {
    state.diff_viewer.is_some()
}
