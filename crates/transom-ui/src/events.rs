//! 应用顶层事件分发与终端事件回调处理模块。

use crate::app::{AppShell, TabStatus};
use crate::palette::localized_error;
use gpui_kit::*;
use transom_core::{AppEvent, TabId};
use transom_terminal::TerminalEvent;

impl AppShell {
    pub(crate) fn handle_app_event(
        &mut self,
        event: AppEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            AppEvent::TerminalReady { .. } => {
                if !self.on_terminal_ready(event, window, cx) {
                    return;
                }
            }
            AppEvent::ConnectionFailed { .. } => {
                if !self.on_connection_failed(event, cx) {
                    return;
                }
            }
            AppEvent::Disconnected { .. } => {
                if !self.on_disconnected(event, cx) {
                    return;
                }
            }
            AppEvent::AuthPrompt { .. } => {
                if !self.on_auth_prompt(event, window, cx) {
                    return;
                }
            }
            AppEvent::HostKeyConfirm { .. } => {
                if !self.on_host_key_confirm(event) {
                    return;
                }
            }
            AppEvent::SftpListing { .. } => {
                if !self.on_sftp_listing(event, window, cx) {
                    return;
                }
            }
            AppEvent::SftpProgress { .. } => {
                if !self.on_sftp_progress(event, cx) {
                    return;
                }
            }
            AppEvent::SftpFinished { .. } => {
                if !self.on_sftp_finished(event, cx) {
                    return;
                }
            }
            AppEvent::SftpMetadata { .. } => {
                if !self.on_sftp_metadata(event, cx) {
                    return;
                }
            }
            AppEvent::SftpPreview { .. } => {
                if !self.on_sftp_preview(event, cx) {
                    return;
                }
            }
            AppEvent::SftpPreviewFailed { .. } => {
                if !self.on_sftp_preview_failed(event, cx) {
                    return;
                }
            }
            AppEvent::SftpTextRead {
                op_id,
                content,
                meta,
                ..
            } => {
                self.apply_editor_text(op_id, content, meta, window, cx);
            }
            AppEvent::SftpTextWritten { op_id, meta, .. } => {
                self.note_editor_text_written(op_id, meta, window, cx);
            }
            AppEvent::SftpTextFailed { op_id, error, .. } => {
                // 编辑读写失败只回到面板内显示，不写远端栏错误、不进传输队列统计。
                self.note_editor_text_failed(op_id, localized_error(&error), cx);
            }
            AppEvent::SftpTextConflict { op_id, error, .. } => {
                // 版本指纹拦截不是普通失败：面板要弹出三选一冲突横幅。
                self.note_editor_text_conflict(op_id, localized_error(&error), cx);
            }
            AppEvent::SftpDirCompared { .. } => {
                if !self.on_sftp_dir_compared(event) {
                    return;
                }
            }
            AppEvent::SftpFailed { .. } => {
                if !self.on_sftp_failed(event, cx) {
                    return;
                }
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
            AppEvent::SystemStatsUpdated { .. } => {
                if !self.on_system_stats(event, cx) {
                    return;
                }
            }
            AppEvent::ProcessesSampled {
                tab_id,
                connection_id,
                processes,
            } => {
                self.monitor_sampled(tab_id, connection_id, processes, cx);
                return;
            }
            AppEvent::ProcessesSampleFailed {
                tab_id,
                connection_id,
                error,
            } => {
                self.monitor_failed(tab_id, connection_id, error, cx);
                return;
            }
            AppEvent::ProcessKilled {
                tab_id,
                connection_id,
                pid,
                error,
            } => {
                self.monitor_killed(tab_id, connection_id, pid, error, cx);
                return;
            }
            AppEvent::NetworkTrafficUpdated { .. } => {
                if !self.on_network_traffic(event, cx) {
                    return;
                }
            }
            AppEvent::PublicKeyDeployed {
                tab_id: _,
                connection_id: _,
                key_name,
                error,
            } => {
                self.handle_public_key_deployed(key_name, error, cx);
            }
            AppEvent::CloudSyncPushed { result } => {
                self.handle_cloud_sync_pushed(result, cx);
                return;
            }
            AppEvent::Zmodem {
                tab_id,
                connection_id: _,
                event,
            } => {
                self.handle_zmodem_event(tab_id, event, cx);
                return;
            }
            AppEvent::GitSynced { result } => {
                self.handle_git_synced(result, cx);
                return;
            }
            AppEvent::CloudSyncPulled { result } => {
                match result {
                    Ok(document) => self.finish_cloud_sync_pull(document, window, cx),
                    Err(error) if error.code == transom_core::ErrorCode::CloudSyncMissing => {
                        self.handle_cloud_sync_missing(cx);
                    }
                    Err(error) => {
                        self.error = Some(localized_error(&error));
                    }
                }
                return;
            }
            AppEvent::HostActionResult {
                session_id,
                outcome,
            } => {
                self.handle_probe_result(session_id, outcome, cx);
                return;
            }
            AppEvent::DeepLink { request } => {
                self.handle_deep_link(request, window, cx);
                return;
            }
        }
        cx.notify();
    }

    pub(crate) fn handle_terminal_event(
        &mut self,
        tab_id: TabId,
        event: &TerminalEvent,
        window: &mut Window,
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
            TerminalEvent::CommandRecorded { command } => {
                self.record_history_command(command, cx);
            }
            TerminalEvent::FilesDropped(paths) => {
                self.handle_dropped_files(tab_id, paths, window, cx);
            }
            TerminalEvent::CommandFinished {
                exit_code,
                duration_ms,
            } => {
                self.on_command_finished(tab_id, *exit_code, *duration_ms, window, cx);
            }
            TerminalEvent::DesktopNotification { title, body } => {
                self.on_desktop_notification(title, body, cx);
            }
            TerminalEvent::TriggerFired { rule_name, action } => match action {
                transom_core::TriggerAction::AutoResponse(reply) => {
                    self.status = Some(
                        rust_i18n::t!(
                            "trigger.auto_response_notice",
                            name = rule_name,
                            reply = reply
                        )
                        .to_string(),
                    );
                }
                transom_core::TriggerAction::Bell => {
                    self.status = Some(rust_i18n::t!("status.bell").to_string());
                }
                _ => {}
            },
        }
        cx.notify();
    }
}
