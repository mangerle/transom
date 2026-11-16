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
            AppEvent::Container(container_event) => {
                crate::container::dispatch_container_event(
                    &mut self.state,
                    container_event,
                    window,
                    cx,
                );
            }
            AppEvent::Git(git_event) => {
                crate::git::actions::dispatch_git_event(&mut self.state, git_event, cx);
            }
            AppEvent::DeepLink { request } => {
                crate::connection::handle_deep_link(&mut self.state, request, window, cx);
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
            TerminalAppEvent::HostKeyConfirm { .. } => self.on_host_key_confirm(event, cx),
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
            SftpAppEvent::SftpListing { .. } => {
                crate::sftp::on_sftp_listing(&mut self.state, event, window, cx)
            }
            SftpAppEvent::SftpProgress { .. } => {
                crate::sftp::on_sftp_progress(&mut self.state, event, cx)
            }
            SftpAppEvent::SftpVerifying { .. } => {
                crate::sftp::on_sftp_verifying(&mut self.state, event, cx)
            }
            SftpAppEvent::SftpRetrying { .. } => {
                crate::sftp::on_sftp_retrying(&mut self.state, event, cx)
            }
            SftpAppEvent::SftpFinished { .. } => {
                crate::sftp::on_sftp_finished(&mut self.state, event, cx)
            }
            SftpAppEvent::SftpMetadata { .. } => {
                crate::sftp::on_sftp_metadata(&mut self.state, event, cx)
            }
            SftpAppEvent::SftpPreview { .. } => {
                crate::sftp::on_sftp_preview(&mut self.state, event, cx)
            }
            SftpAppEvent::SftpPreviewFailed { .. } => {
                crate::sftp::on_sftp_preview_failed(&mut self.state, event, cx)
            }
            SftpAppEvent::SftpTextRead {
                op_id,
                content,
                meta,
                ..
            } => {
                // 对比面板与编辑器共用同一条只读通道，按在途请求归属分发。
                if !crate::diff_viewer::DiffViewerOps::new(&mut self.state)
                    .handle_remote_read(op_id, &content, meta, cx)
                {
                    self.state.features.editors.update(cx, |panels, cx| {
                        panels.apply_editor_text(op_id, content, meta, window, cx)
                    });
                }
                true
            }
            SftpAppEvent::SftpTextWritten { op_id, meta, .. } => {
                if !crate::diff_viewer::DiffViewerOps::new(&mut self.state)
                    .handle_remote_written(op_id, meta, window, cx)
                {
                    crate::editor::on_editor_text_written(&mut self.state, op_id, meta, window, cx);
                }
                true
            }
            SftpAppEvent::SftpTextFailed { op_id, error, .. } => {
                // 编辑读写失败只回到面板内显示，不写远端栏错误、不进传输队列统计。
                let code = error.code;
                let message = localized_error(&error);
                if !crate::diff_viewer::DiffViewerOps::new(&mut self.state)
                    .handle_remote_failure(op_id, &message, false, cx)
                {
                    crate::editor::note_editor_text_failed(
                        &mut self.state,
                        op_id,
                        message,
                        code,
                        window,
                        cx,
                    );
                }
                true
            }
            SftpAppEvent::SftpTextConflict { op_id, error, .. } => {
                // 版本指纹拦截不是普通失败：面板要弹出三选一冲突横幅。
                let message = localized_error(&error);
                if !crate::diff_viewer::DiffViewerOps::new(&mut self.state)
                    .handle_remote_failure(op_id, &message, true, cx)
                {
                    crate::editor::note_editor_text_conflict(&mut self.state, op_id, message, cx);
                }
                true
            }
            SftpAppEvent::SftpSearchResults { items, .. } => {
                if let Some(modal) = self.state.ui.modals.sftp.search.clone() {
                    modal.update(cx, |m, cx| m.apply_results(items, None, cx));
                }
                true
            }
            SftpAppEvent::SftpDirCompared { .. } => {
                crate::sftp::on_sftp_dir_compared(&mut self.state, event, cx)
            }
            SftpAppEvent::SudoProbed {
                op_id, capability, ..
            } => {
                crate::editor::on_sudo_probed(&mut self.state, op_id, capability, window, cx);
                true
            }
            SftpAppEvent::SftpFailed { .. } => {
                crate::sftp::on_sftp_failed(&mut self.state, event, cx)
            }
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
                    .state
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
                    self.state.set_error(message);
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
                crate::monitor::MonitorOps::new(&mut self.state).monitor_sampled(
                    tab_id,
                    connection_id,
                    processes,
                    cx,
                );
            }
            HostAppEvent::ProcessCountSampled {
                tab_id,
                connection_id,
                count,
            } => {
                crate::monitor::MonitorOps::new(&mut self.state).monitor_count_sampled(
                    tab_id,
                    connection_id,
                    count,
                    cx,
                );
            }
            HostAppEvent::ProcessesSampleFailed {
                tab_id,
                connection_id,
                error,
            } => {
                crate::monitor::MonitorOps::new(&mut self.state).monitor_failed(
                    tab_id,
                    connection_id,
                    error,
                    cx,
                );
            }
            HostAppEvent::ProcessKilled {
                tab_id,
                connection_id,
                pid,
                error,
            } => {
                crate::monitor::MonitorOps::new(&mut self.state).monitor_killed(
                    tab_id,
                    connection_id,
                    pid,
                    error,
                    cx,
                );
            }
            HostAppEvent::PublicKeyDeployed {
                tab_id: _,
                connection_id: _,
                key_name,
                error,
            } => {
                crate::keys::handle_public_key_deployed(self, key_name, error, cx);
                cx.notify();
            }
            HostAppEvent::HostActionResult {
                session_id,
                outcome,
            } => {
                crate::probe::handle_probe_result(self, session_id, outcome, cx);
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
        crate::cloud_sync::handle_sync_app_event(&mut self.state, event, window, cx);
    }

    /// ZMODEM 传输事件分发。
    fn dispatch_zmodem_event(&mut self, event: ZmodemAppEvent, cx: &mut Context<Self>) {
        let ZmodemAppEvent::Zmodem {
            tab_id,
            connection_id: _,
            event,
        } = event;
        crate::zmodem::handle_zmodem_event(self, tab_id, event, cx);
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
                if let Some(tab) = self.state.workspace.tab_mut(tab_id) {
                    tab.title = title.clone();
                }
            }
            // 标题栈弹出或终端要求恢复默认标题：运行期标题清空，标签回落会话名。
            TerminalEvent::ResetTitle => {
                if let Some(tab) = self.state.workspace.tab_mut(tab_id) {
                    tab.title.clear();
                }
            }
            TerminalEvent::Cwd(path) => {
                log::debug!("收到终端 OSC 7 cwd，窗格: {tab_id}，路径: {path}");
                if let Some(tab) = self.state.workspace.tab_mut(tab_id) {
                    tab.cwd = Some(path.clone());
                }
                crate::sftp::sftp_sync_with_terminal_cwd(&mut self.state, tab_id, path, window, cx);
            }
            TerminalEvent::Bell => {
                self.set_transient_status(rust_i18n::t!("status.bell"), cx);
            }
            TerminalEvent::ClipboardStored => {
                self.set_transient_status(rust_i18n::t!("status.copied"), cx);
            }
            TerminalEvent::ClipboardWriteRequest { text } => {
                crate::connection::request_clipboard_permission(
                    &mut self.state,
                    tab_id,
                    text.clone(),
                    cx,
                );
            }
            TerminalEvent::CopyModeChanged { active } => {
                // 提示只在本次进入时写入；退出时只清理自己写的那条，避免覆盖
                // 期间产生的其他状态消息。进入与退出可能来自按键、鼠标或复制
                // 动作，统一在这里响应才不会漏掉某个出口。
                let hint = rust_i18n::t!("copy_mode.hint").to_string();
                if *active {
                    self.state.ui.status = Some(hint);
                } else if self.state.ui.status.as_deref() == Some(hint.as_str()) {
                    self.state.ui.status = None;
                }
            }
            TerminalEvent::Exit => {
                if let Some(tab) = self.state.workspace.tab_mut(tab_id)
                    && !matches!(tab.status, TabStatus::Failed(_))
                {
                    tab.status = TabStatus::Disconnected;
                }
            }
            TerminalEvent::ContextMenuRequested { .. } => {}
            TerminalEvent::CommandRecorded { command } => {
                crate::connection::record_history_command(&mut self.state, command, cx);
            }
            TerminalEvent::FilesDropped(paths) => {
                crate::sftp::handle_dropped_files(&mut self.state, tab_id, paths, window, cx);
            }
            TerminalEvent::CommandFinished {
                exit_code,
                duration_ms,
            } => {
                crate::notify::on_command_finished(
                    self,
                    tab_id,
                    *exit_code,
                    *duration_ms,
                    window,
                    cx,
                );
            }
            TerminalEvent::DesktopNotification { title, body } => {
                crate::notify::on_desktop_notification(self, title, body, cx);
            }
            TerminalEvent::OpenCodeLocation { path, line, column } => {
                crate::editor::open_terminal_code_location(
                    &mut self.state,
                    tab_id,
                    path,
                    *line,
                    *column,
                    window,
                    cx,
                );
            }
            TerminalEvent::TriggerFired { rule_name, action } => match action {
                transom_core::TriggerAction::AutoResponse(reply) => {
                    self.state.ui.status = Some(
                        rust_i18n::t!(
                            "trigger.auto_response_notice",
                            name = rule_name,
                            reply = reply
                        )
                        .to_string(),
                    );
                }
                transom_core::TriggerAction::Bell => {
                    self.state.ui.status = Some(rust_i18n::t!("status.bell").to_string());
                }
                _ => {}
            },
        }
        cx.notify();
    }
}

/// 已知主机弹窗上报的删除结果：成功写状态栏，失败进应用通知队列。
pub(crate) fn on_known_hosts_event(
    shell: &mut AppShell,
    event: &crate::keys::KnownHostsEvent,
    cx: &mut Context<AppShell>,
) {
    match event {
        crate::keys::KnownHostsEvent::Removed { error: Some(error) } => shell
            .state
            .set_error(crate::palette::localized_error(error)),
        crate::keys::KnownHostsEvent::Removed { error: None } => {
            shell.set_transient_status(rust_i18n::t!("known_hosts.deleted"), cx);
        }
    }
    cx.notify();
}
