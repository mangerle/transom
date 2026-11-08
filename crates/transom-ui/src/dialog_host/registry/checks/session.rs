//! 会话、连接、工具、SFTP、编辑器与差异视图的弹窗判定。
use crate::app::AppShell;
use gpui_kit::App;

pub(in crate::app::dialog_host) fn session_form_open(shell: &AppShell, _: &App) -> bool {
    shell.state.modals.session.form.is_some()
}

pub(in crate::app::dialog_host) fn session_form_values(shell: &AppShell, cx: &App) -> Vec<String> {
    shell
        .state
        .modals
        .session
        .form
        .as_ref()
        .map(|form| form.text_values(cx))
        .unwrap_or_default()
}

pub(in crate::app::dialog_host) fn folder_form_open(shell: &AppShell, _: &App) -> bool {
    shell.state.modals.session.folder_form.is_some()
}

pub(in crate::app::dialog_host) fn folder_form_values(shell: &AppShell, cx: &App) -> Vec<String> {
    shell
        .state
        .modals
        .session
        .folder_form
        .as_ref()
        .map(|form| form.text_values(cx))
        .unwrap_or_default()
}

pub(in crate::app::dialog_host) fn batch_edit_open(shell: &AppShell, _: &App) -> bool {
    shell.state.modals.session.batch_edit.is_some()
}

pub(in crate::app::dialog_host) fn batch_edit_values(shell: &AppShell, cx: &App) -> Vec<String> {
    shell
        .state
        .modals
        .session
        .batch_edit
        .as_ref()
        .map(|form| form.text_values(cx))
        .unwrap_or_default()
}

pub(in crate::app::dialog_host) fn auth_prompt_open(shell: &AppShell, _: &App) -> bool {
    shell.state.modals.connection.auth_prompt.is_some()
}

pub(in crate::app::dialog_host) fn host_key_confirm_open(shell: &AppShell, _: &App) -> bool {
    shell.state.modals.connection.host_key_confirm.is_some()
}

pub(in crate::app::dialog_host) fn clipboard_prompt_open(shell: &AppShell, _: &App) -> bool {
    shell.state.modals.connection.clipboard_request.is_some()
}

pub(in crate::app::dialog_host) fn known_hosts_open(shell: &AppShell, cx: &App) -> bool {
    shell.state.modals.connection.known_hosts.read(cx).is_open()
}

pub(in crate::app::dialog_host) fn forward_confirm_open(shell: &AppShell, _: &App) -> bool {
    shell.state.modals.tools.confirm_forward_any.is_some()
}

pub(in crate::app::dialog_host) fn key_wizard_open(shell: &AppShell, cx: &App) -> bool {
    shell.state.modals.tools.key_wizard.read(cx).is_open()
}

pub(in crate::app::dialog_host) fn backup_prompt_open(shell: &AppShell, cx: &App) -> bool {
    shell.state.modals.tools.backup_prompt.read(cx).is_open()
}

pub(in crate::app::dialog_host) fn audit_viewer_open(shell: &AppShell, cx: &App) -> bool {
    shell.state.modals.tools.audit_viewer.read(cx).is_open()
}

pub(in crate::app::dialog_host) fn replay_open(shell: &AppShell, cx: &App) -> bool {
    shell.state.modals.tools.replay.read(cx).is_open()
}

pub(in crate::app::dialog_host) fn snippet_form_open(shell: &AppShell, _: &App) -> bool {
    shell.state.modals.tools.snippet_panel.form.is_some()
}

pub(in crate::app::dialog_host) fn snippet_form_values(shell: &AppShell, cx: &App) -> Vec<String> {
    shell
        .state
        .modals
        .tools
        .snippet_panel
        .form
        .as_ref()
        .map(|form| form.text_values(cx))
        .unwrap_or_default()
}

pub(in crate::app::dialog_host) fn settings_open(shell: &AppShell, cx: &App) -> bool {
    shell.state.modals.settings.read(cx).is_open()
}

pub(in crate::app::dialog_host) fn shortcut_capture_open(shell: &AppShell, _: &App) -> bool {
    shell.state.shortcut_recording.capture.is_some()
}

// ---- SFTP ----

pub(in crate::app::dialog_host) fn sftp_confirm_open(shell: &AppShell, _: &App) -> bool {
    shell.state.sftp_modals.confirm.is_some()
}

pub(in crate::app::dialog_host) fn sftp_input_open(shell: &AppShell, _: &App) -> bool {
    shell.state.sftp_modals.input.is_some()
}

pub(in crate::app::dialog_host) fn sftp_chmod_open(shell: &AppShell, _: &App) -> bool {
    shell.state.sftp_modals.chmod.is_some()
}

pub(in crate::app::dialog_host) fn sftp_properties_open(shell: &AppShell, _: &App) -> bool {
    shell.state.sftp_modals.properties.is_some()
}

pub(in crate::app::dialog_host) fn sftp_filter_open(shell: &AppShell, _: &App) -> bool {
    shell.state.sftp_modals.filter.is_some()
}

pub(in crate::app::dialog_host) fn sftp_search_open(shell: &AppShell, _: &App) -> bool {
    shell.state.sftp_modals.search.is_some()
}

pub(in crate::app::dialog_host) fn sftp_sync_open(shell: &AppShell, _: &App) -> bool {
    shell.state.sftp_modals.sync.is_some()
}

pub(in crate::app::dialog_host) fn editor_goto_line_open(shell: &AppShell, _: &App) -> bool {
    shell.state.sftp_modals.goto_line.is_some()
}

pub(in crate::app::dialog_host) fn diff_viewer_open(shell: &AppShell, _: &App) -> bool {
    shell.state.diff_viewer.is_some()
}
