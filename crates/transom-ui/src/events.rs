//! 应用顶层事件分发与终端事件回调处理模块。

use crate::app::{AppShell, AuthPromptState, TabStatus};
use crate::keymap;
use crate::palette::localized_error;
use crate::sftp::{
    SftpEditorModalState, SftpPropertiesModalState, format_size, remote_file_properties,
    send_desktop_notification,
};
use gpui_kit::component::input::{InputState, TextareaState};
use gpui_kit::*;
use std::sync::Arc;
use transom_core::{AppEvent, TabId, TransferDirection, TransferStatus};
use transom_terminal::{KeyFilter, TerminalEvent, TerminalView};

impl AppShell {
    pub(crate) fn handle_app_event(
        &mut self,
        event: AppEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            AppEvent::TerminalReady {
                tab_id,
                connection_id,
                io,
            } => {
                let Some(index) = self.tabs.iter().position(|tab| tab.id == tab_id) else {
                    return;
                };
                if self.tabs[index].connection_id != Some(connection_id) {
                    return;
                }
                let session_id = self.tabs[index].session_id;
                let profile = self
                    .sessions
                    .iter()
                    .find(|session| session.id == session_id)
                    .map(|session| session.terminal.clone())
                    .unwrap_or_else(|| self.tabs[index].session.terminal.clone());
                let interaction = self.settings.terminal_interaction;
                let key_filter: KeyFilter = Arc::new(keymap::is_app_shortcut);
                let terminal = cx.new(|cx| {
                    TerminalView::new(window, cx, io, profile, interaction, Some(key_filter))
                });
                let focus_handle = terminal.read(cx).focus_handle().clone();
                let subscription =
                    cx.subscribe(&terminal, move |this, _, event: &TerminalEvent, cx| {
                        this.handle_terminal_event(tab_id, event, cx)
                    });
                let tab = &mut self.tabs[index];
                tab.terminal = Some(terminal);
                tab.status = TabStatus::Connected;
                tab._subscription = Some(subscription);
                self.active_tab = Some(tab_id);
                window.focus(&focus_handle, cx);
                self.auto_start_forwards(tab_id, connection_id, session_id, cx);
            }
            AppEvent::ConnectionFailed {
                tab_id,
                connection_id,
                error,
            } => {
                if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id)
                    && tab.connection_id == Some(connection_id)
                {
                    tab.status = TabStatus::Failed(error.clone());
                    self.error = Some(localized_error(&error));
                }
                self.mark_forwards_stopped(tab_id);
            }
            AppEvent::Disconnected {
                tab_id,
                connection_id,
            } => {
                if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id)
                    && tab.connection_id == Some(connection_id)
                    && !matches!(tab.status, TabStatus::Failed(_))
                {
                    tab.status = TabStatus::Disconnected;
                }
                self.mark_forwards_stopped(tab_id);
            }
            AppEvent::AuthPrompt {
                tab_id,
                connection_id,
                name,
                instructions,
                prompts,
                responder,
            } => {
                let Some(tab) = self.tabs.iter().find(|tab| tab.id == tab_id) else {
                    return;
                };
                if tab.connection_id != Some(connection_id) {
                    return;
                }
                let inputs = prompts
                    .iter()
                    .map(|prompt| {
                        let placeholder = prompt.prompt.clone();
                        let masked = !prompt.echo;
                        cx.new(|cx| {
                            let state = InputState::new(window, cx).placeholder(placeholder);
                            if masked { state.masked(true) } else { state }
                        })
                    })
                    .collect();
                self.auth_prompt = Some(AuthPromptState {
                    name,
                    instructions,
                    prompts,
                    inputs,
                    responder,
                    error: None,
                });
            }
            AppEvent::SftpListing {
                tab_id,
                connection_id,
                path,
                entries,
            } => {
                let mut update_input = None;
                if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id)
                    && tab.connection_id == Some(connection_id)
                    && let Some(panel) = tab.sftp.as_mut()
                {
                    panel.path = path.clone();
                    panel.entries = entries;
                    panel.loading = false;
                    panel.error = None;
                    if let Some(selected) = &panel.selected
                        && !panel.entries.iter().any(|entry| &entry.path == selected)
                    {
                        panel.selected = None;
                    }
                    update_input = Some((panel.path_input.clone(), path));
                }
                if let Some((input, path)) = update_input {
                    input.update(cx, |state, cx| state.set_value(path, window, cx));
                }
            }
            AppEvent::SftpProgress {
                tab_id,
                connection_id,
                op_id,
                transferred,
                total,
            } => {
                if let Some(panel) = self.sftp_panel_mut(tab_id, connection_id) {
                    if panel.active_op == Some(op_id) {
                        panel.progress = Some((transferred, total));
                    }
                    if let Some(task) = panel.queue.iter_mut().find(|t| t.id == op_id) {
                        task.transferred = transferred;
                        if total.is_some() {
                            task.size = total;
                        }
                    }
                }
            }
            AppEvent::SftpFinished {
                tab_id,
                connection_id,
                op_id,
                transferred,
                skipped,
                verified,
            } => {
                let mut refresh = None;
                let mut refresh_local = false;
                let mut open_editor_info = None;
                let mut notify_info: Option<(TransferDirection, String, u64)> = None;
                if let Some(panel) = self.sftp_panel_mut(tab_id, connection_id) {
                    if let Some(info) = panel.pending_edit_ops.remove(&op_id) {
                        open_editor_info = Some(info);
                    }
                    if let Some(task) = panel.queue.iter_mut().find(|t| t.id == op_id) {
                        task.status = TransferStatus::Completed;
                        if let Some(bytes) = transferred {
                            task.transferred = bytes;
                        }
                        if task.direction == TransferDirection::Download
                            || task.direction == TransferDirection::Sync
                        {
                            refresh_local = true;
                        }
                        if open_editor_info.is_none() {
                            let file_name = task
                                .local_path
                                .file_name()
                                .map(|n| n.to_string_lossy().to_string())
                                .unwrap_or_else(|| task.remote_path.clone());
                            notify_info = Some((task.direction, file_name, task.transferred));
                        }
                    }
                    if panel.active_op == Some(op_id) || panel.active_op.is_none() {
                        panel.active_op = None;
                        panel.progress = None;
                        let base = match transferred {
                            Some(bytes) => format!(
                                "{} ({})",
                                rust_i18n::t!("sftp.transfer_done"),
                                format_size(bytes)
                            ),
                            None => rust_i18n::t!("sftp.operation_done").to_string(),
                        };
                        let mut message = base;
                        if skipped > 0 {
                            message.push_str(&format!(
                                " | {}: {skipped}",
                                rust_i18n::t!("sftp.skipped_items")
                            ));
                        }
                        if verified {
                            message
                                .push_str(&format!(" | {}", rust_i18n::t!("sftp.sha256_verified")));
                        }
                        panel.status = Some(message);
                        refresh = Some(panel.path.clone());
                    }
                }
                if let Some((dir, file_name, bytes)) = notify_info {
                    let action = match dir {
                        TransferDirection::Upload => "上传完成",
                        TransferDirection::Download => "下载完成",
                        TransferDirection::Sync => "同步完成",
                    };
                    send_desktop_notification(
                        &format!("Transom SFTP {action}"),
                        &format!("{file_name} ({})", format_size(bytes)),
                    );
                }
                if let Some((local_path, remote_path, file_name)) = open_editor_info {
                    match std::fs::read_to_string(&local_path) {
                        Ok(content) => {
                            let textarea =
                                cx.new(|cx| TextareaState::new(window, cx).default_value(content));
                            self.sftp_editor_modal = Some(SftpEditorModalState {
                                tab_id,
                                local_path,
                                remote_path,
                                file_name,
                                textarea,
                                error: None,
                            });
                        }
                        Err(err) => {
                            if let Some(panel) = self.sftp_panel_mut(tab_id, connection_id) {
                                panel.error = Some(format!(
                                    "读取文本内容失败（可能非 UTF-8 文本文件）: {err}"
                                ));
                            }
                        }
                    }
                }
                if refresh_local {
                    self.sftp_local_refresh(tab_id, cx);
                }
                if let Some(path) = refresh {
                    self.request_sftp_list(tab_id, path, cx);
                }
                self.process_sftp_queue(tab_id, cx);
            }
            AppEvent::SftpMetadata {
                tab_id,
                connection_id,
                op_id,
                metadata,
            } => {
                let Some(panel) = self.sftp_panel_mut(tab_id, connection_id) else {
                    // 标签页已关闭（或连接已切换）：忽略迟到的属性响应，不再弹出属性窗
                    return;
                };
                if panel.active_op == Some(op_id) {
                    panel.active_op = None;
                    panel.progress = None;
                }
                self.sftp_properties_modal = Some(SftpPropertiesModalState {
                    scope: "远端".to_string(),
                    properties: remote_file_properties(&metadata),
                });
                self.process_sftp_queue(tab_id, cx);
            }
            AppEvent::SftpDirCompared {
                tab_id: _,
                connection_id,
                op_id,
                comparison,
            } => {
                if let Some(modal) = self.sftp_sync_modal.as_mut()
                    && modal.op_id == op_id
                    && modal.connection_id == connection_id
                {
                    modal.scanning = false;
                    modal.skipped = comparison.skipped;
                    modal.selected = comparison
                        .entries
                        .iter()
                        .map(|entry| entry.sync_item().is_some())
                        .collect();
                    modal.entries = comparison.entries;
                    modal.error = None;
                }
            }
            AppEvent::SftpFailed {
                tab_id,
                connection_id,
                op_id,
                error,
            } => {
                if let Some(modal) = self.sftp_sync_modal.as_mut()
                    && modal.op_id == op_id
                    && modal.connection_id == connection_id
                {
                    modal.scanning = false;
                    modal.error = Some(localized_error(&error));
                    cx.notify();
                    return;
                }
                let mut failed_notify: Option<(String, String)> = None;
                if let Some(panel) = self.sftp_panel_mut(tab_id, connection_id) {
                    panel.pending_edit_ops.remove(&op_id);
                    let err_msg = localized_error(&error);
                    if let Some(task) = panel.queue.iter_mut().find(|t| t.id == op_id) {
                        task.status = TransferStatus::Failed(err_msg.clone());
                        let file_name = task
                            .local_path
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_else(|| task.remote_path.clone());
                        failed_notify = Some((file_name, err_msg.clone()));
                    }
                    if panel.active_op == Some(op_id) || panel.active_op.is_none() {
                        panel.active_op = None;
                        panel.progress = None;
                        panel.loading = false;
                        panel.error = Some(err_msg);
                    }
                }
                if let Some((file_name, err_msg)) = failed_notify {
                    send_desktop_notification(
                        "Transom SFTP 传输失败",
                        &format!("{file_name}: {err_msg}"),
                    );
                }
                self.process_sftp_queue(tab_id, cx);
            }
            AppEvent::ForwardChanged {
                tab_id,
                connection_id,
                status,
            } => {
                if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id)
                    && tab.connection_id == Some(connection_id)
                {
                    if let Some(error) = &status.error {
                        self.error = Some(localized_error(error));
                    }
                    tab.forward_statuses.insert(status.rule_id, status);
                }
            }
            AppEvent::SystemStatsUpdated {
                tab_id,
                connection_id,
                stats,
            } => {
                if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id)
                    && tab.connection_id == Some(connection_id)
                {
                    tab.system_stats = Some(stats);
                }
            }
        }
        cx.notify();
    }

    pub(crate) fn handle_terminal_event(
        &mut self,
        tab_id: TabId,
        event: &TerminalEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            TerminalEvent::Title(title) => {
                if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id) {
                    tab.title = title.clone();
                }
                self.sftp_sync_with_terminal_title(tab_id, title, cx);
            }
            TerminalEvent::ResetTitle => {}
            TerminalEvent::Bell => {
                self.status = Some(rust_i18n::t!("status.bell").to_string());
            }
            TerminalEvent::ClipboardStored => {
                self.status = Some(rust_i18n::t!("status.copied").to_string());
            }
            TerminalEvent::Exit => {
                if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id)
                    && !matches!(tab.status, TabStatus::Failed(_))
                {
                    tab.status = TabStatus::Disconnected;
                }
            }
            TerminalEvent::ContextMenuRequested { .. } => {}
        }
        cx.notify();
    }
}
