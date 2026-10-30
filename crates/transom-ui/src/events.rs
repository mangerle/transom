//! 应用顶层事件分发与终端事件回调处理模块。

use crate::app::{AppShell, TabStatus};
use crate::palette::localized_error;
use gpui_kit::*;
use transom_core::{
    AppEvent, ForwardAppEvent, HostAppEvent, SftpAppEvent, SyncAppEvent, TabId, TerminalAppEvent,
    ZmodemAppEvent,
};
use transom_terminal::TerminalEvent;

impl AppShell {
    /// 应用顶层事件总线分发入口：按领域分发给对应的子系统处理器。
    pub(crate) fn handle_app_event(
        &mut self,
        event: AppEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            AppEvent::Terminal(terminal_event) => {
                self.dispatch_terminal_event(terminal_event, window, cx);
            }
            AppEvent::Sftp(sftp_event) => {
                self.dispatch_sftp_event(sftp_event, window, cx);
            }
            AppEvent::Forward(forward_event) => {
                self.dispatch_forward_event(forward_event, cx);
            }
            AppEvent::Host(host_event) => {
                self.dispatch_host_event(host_event, cx);
            }
            AppEvent::Sync(sync_event) => {
                self.dispatch_sync_event(sync_event, window, cx);
            }
            AppEvent::Zmodem(zmodem_event) => {
                self.dispatch_zmodem_event(zmodem_event, cx);
            }
            AppEvent::DeepLink { request } => {
                self.handle_deep_link(request, window, cx);
            }
        }
    }

    /// 终端会话与连接事件分发。
    fn dispatch_terminal_event(
        &mut self,
        event: TerminalAppEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let should_notify = match event {
            TerminalAppEvent::TerminalReady { .. } => self.on_terminal_ready(event, window, cx),
            TerminalAppEvent::ConnectionFailed { .. } => self.on_connection_failed(event, cx),
            TerminalAppEvent::Disconnected { .. } => self.on_disconnected(event, cx),
            TerminalAppEvent::AuthPrompt { .. } => self.on_auth_prompt(event, window, cx),
            TerminalAppEvent::HostKeyConfirm { .. } => self.on_host_key_confirm(event),
        };
        if should_notify {
            cx.notify();
        }
    }

    /// SFTP 文件传输与文本编辑事件分发。
    fn dispatch_sftp_event(
        &mut self,
        event: SftpAppEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let should_notify = match event {
            SftpAppEvent::SftpListing { .. } => self.on_sftp_listing(event, window, cx),
            SftpAppEvent::SftpProgress { .. } => self.on_sftp_progress(event, cx),
            SftpAppEvent::SftpVerifying { .. } => self.on_sftp_verifying(event, cx),
            SftpAppEvent::SftpFinished { .. } => self.on_sftp_finished(event, cx),
            SftpAppEvent::SftpMetadata { .. } => self.on_sftp_metadata(event, cx),
            SftpAppEvent::SftpPreview { .. } => self.on_sftp_preview(event, cx),
            SftpAppEvent::SftpPreviewFailed { .. } => self.on_sftp_preview_failed(event, cx),
            SftpAppEvent::SftpTextRead {
                op_id,
                content,
                meta,
                ..
            } => {
                // 对比面板与编辑器共用同一条只读通道，按在途请求归属分发。
                if !self.diff_handle_remote_read(op_id, &content, meta, cx) {
                    self.apply_editor_text(op_id, content, meta, window, cx);
                }
                true
            }
            SftpAppEvent::SftpTextWritten { op_id, meta, .. } => {
                if !self.diff_handle_remote_written(op_id, meta, window, cx) {
                    self.note_editor_text_written(op_id, meta, window, cx);
                }
                true
            }
            SftpAppEvent::SftpTextFailed { op_id, error, .. } => {
                // 编辑读写失败只回到面板内显示，不写远端栏错误、不进传输队列统计。
                let code = error.code;
                let message = localized_error(&error);
                if !self.diff_handle_remote_failure(op_id, &message, false, cx) {
                    self.note_editor_text_failed(op_id, message, code, window, cx);
                }
                true
            }
            SftpAppEvent::SftpTextConflict { op_id, error, .. } => {
                // 版本指纹拦截不是普通失败：面板要弹出三选一冲突横幅。
                let message = localized_error(&error);
                if !self.diff_handle_remote_failure(op_id, &message, true, cx) {
                    self.note_editor_text_conflict(op_id, message, cx);
                }
                true
            }
            SftpAppEvent::SftpSearchResults { items, .. } => {
                if let Some(ref mut modal) = self.sftp_modals.search {
                    modal.is_searching = false;
                    modal.results = items;
                }
                true
            }
            SftpAppEvent::SftpDirCompared { .. } => self.on_sftp_dir_compared(event),
            SftpAppEvent::SudoProbed {
                op_id, capability, ..
            } => {
                self.on_sudo_probed(op_id, capability, window, cx);
                true
            }
            SftpAppEvent::SftpFailed { .. } => self.on_sftp_failed(event, cx),
        };
        if should_notify {
            cx.notify();
        }
    }

    /// 端口转发事件分发。
    fn dispatch_forward_event(&mut self, event: ForwardAppEvent, cx: &mut Context<Self>) {
        match event {
            ForwardAppEvent::ForwardChanged {
                tab_id,
                connection_id,
                status,
            } => {
                let mut error_message = None;
                if let Some(tab) = self
                    .workspace
                    .tabs
                    .iter_mut()
                    .find(|tab| tab.id == tab_id && tab.connection_id == Some(connection_id))
                {
                    error_message = status.error.as_ref().map(localized_error);
                    if let Some(pane) = tab.session_pane_mut() {
                        pane.forward_statuses.insert(status.rule_id, status);
                    }
                }
                // 先结束标签表的可变借用，再入队错误：set_error 需要 &mut self。
                if let Some(message) = error_message {
                    self.set_error(message);
                }
            }
        }
        cx.notify();
    }

    /// 主机遥测、系统采样与运维动作事件分发。
    fn dispatch_host_event(&mut self, event: HostAppEvent, cx: &mut Context<Self>) {
        match event {
            HostAppEvent::SystemStatsUpdated { .. } => {
                self.on_system_stats(event, cx);
            }
            HostAppEvent::NetworkTrafficUpdated { .. } => {
                self.on_network_traffic(event, cx);
            }
            HostAppEvent::ProcessesSampled {
                tab_id,
                connection_id,
                processes,
            } => {
                self.monitor_sampled(tab_id, connection_id, processes, cx);
            }
            HostAppEvent::ProcessCountSampled {
                tab_id,
                connection_id,
                count,
            } => {
                self.monitor_count_sampled(tab_id, connection_id, count, cx);
            }
            HostAppEvent::ProcessesSampleFailed {
                tab_id,
                connection_id,
                error,
            } => {
                self.monitor_failed(tab_id, connection_id, error, cx);
            }
            HostAppEvent::ProcessKilled {
                tab_id,
                connection_id,
                pid,
                error,
            } => {
                self.monitor_killed(tab_id, connection_id, pid, error, cx);
            }
            HostAppEvent::PublicKeyDeployed {
                tab_id: _,
                connection_id: _,
                key_name,
                error,
            } => {
                self.handle_public_key_deployed(key_name, error, cx);
                cx.notify();
            }
            HostAppEvent::HostActionResult {
                session_id,
                outcome,
            } => {
                self.handle_probe_result(session_id, outcome, cx);
            }
        }
    }

    /// 云端备份与 Git 同步事件分发。
    fn dispatch_sync_event(
        &mut self,
        event: SyncAppEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            SyncAppEvent::CloudSyncPushed { result } => {
                self.handle_cloud_sync_pushed(result, cx);
            }
            SyncAppEvent::CloudSyncPulled { result } => match result {
                Ok(document) => self.finish_cloud_sync_pull(document, window, cx),
                Err(error) if error.code == transom_core::ErrorCode::CloudSyncMissing => {
                    self.handle_cloud_sync_missing(cx);
                }
                Err(error) => {
                    self.set_error(localized_error(&error));
                }
            },
            SyncAppEvent::GitSynced { result } => {
                self.handle_git_synced(result, cx);
            }
        }
    }

    /// ZMODEM 传输事件分发。
    fn dispatch_zmodem_event(&mut self, event: ZmodemAppEvent, cx: &mut Context<Self>) {
        let ZmodemAppEvent::Zmodem {
            tab_id,
            connection_id: _,
            event,
        } = event;
        self.handle_zmodem_event(tab_id, event, cx);
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
                if let Some(tab) = self.workspace.tab_mut(tab_id) {
                    tab.title = title.clone();
                }
            }
            // 标题栈弹出或终端要求恢复默认标题：运行期标题清空，标签回落会话名。
            TerminalEvent::ResetTitle => {
                if let Some(tab) = self.workspace.tab_mut(tab_id) {
                    tab.title.clear();
                }
            }
            TerminalEvent::Cwd(path) => {
                log::debug!("收到终端 OSC 7 cwd，窗格: {tab_id}，路径: {path}");
                if let Some(tab) = self.workspace.tab_mut(tab_id) {
                    tab.cwd = Some(path.clone());
                }
                self.sftp_sync_with_terminal_cwd(tab_id, path, window, cx);
            }
            TerminalEvent::Bell => {
                self.status = Some(rust_i18n::t!("status.bell").to_string());
            }
            TerminalEvent::ClipboardStored => {
                self.status = Some(rust_i18n::t!("status.copied").to_string());
            }
            TerminalEvent::ClipboardWriteRequest { text } => {
                self.request_clipboard_permission(tab_id, text.clone(), cx);
            }
            TerminalEvent::Exit => {
                if let Some(tab) = self.workspace.tab_mut(tab_id)
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
            TerminalEvent::OpenCodeLocation { path, line, column } => {
                self.open_terminal_code_location(tab_id, path, *line, *column, window, cx);
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
