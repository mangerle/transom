//! 设置实体事件的跨域处理与渲染期请求消费。
//!
//! 设置实体只改自己领域内的状态；活窗格外观、SFTP 列宽、监控采样、云同步与
//! 文件选择器都属于窗口级动作，集中在这里由外壳执行。处理函数运行在实体
//! 事件派发阶段，**不得回读设置实体**：事件已自带快照，需要窗口的动作排进
//! `window_requests`，由 [`AppShell::flush_window_requests`]
//! 在渲染期执行。

use super::{SettingsEvent, SettingsRequest};
use crate::app::AppShell;
use gpui_kit::*;
use transom_terminal::TerminalView;

impl AppShell {
    /// 设置实体发出的跨域意图。
    pub(crate) fn on_settings_event(&mut self, event: &SettingsEvent, cx: &mut Context<Self>) {
        match event {
            SettingsEvent::Saved(settings) => {
                self.apply_appearance_to_live_panes(cx);
                self.apply_shortcut_settings(cx);
                self.apply_block_view_settings(cx);
                let triggers = settings.triggers.clone();
                let terminals: Vec<Entity<TerminalView>> = self
                    .state
                    .workspace
                    .tabs
                    .iter()
                    .filter_map(|tab| tab.terminal())
                    .collect();
                for terminal in terminals {
                    terminal.update(cx, |view, _| view.reload_triggers(&triggers));
                }
                cx.notify();
            }
            SettingsEvent::Cancelled | SettingsEvent::ProfilePreview => {
                self.apply_appearance_to_live_panes(cx);
                cx.notify();
            }
            SettingsEvent::CatalogChanged(catalog) => {
                self.state.catalog = (**catalog).clone();
                self.state.refresh_tab_sessions();
                self.state.persist_catalog();
                cx.notify();
            }
            SettingsEvent::SetSftpPanelSide(side) => {
                self.set_sftp_panel_side(*side, cx);
            }
            SettingsEvent::MonitorIntervalChanged(mode) => {
                self.set_monitor_interval_mode(*mode, cx);
            }
            SettingsEvent::MonitorSectionsChanged(collapse) => {
                self.state
                    .monitor
                    .update(cx, |view, _| view.state.collapse = *collapse);
                cx.notify();
            }
            SettingsEvent::MonitorDrawerWidthChanged(width) => {
                self.state
                    .monitor
                    .update(cx, |view, _| view.set_drawer_width(*width));
                cx.notify();
            }
            SettingsEvent::TabPositionChanged(position) => {
                self.state.vertical_tabs_open = *position == transom_core::TabPosition::Left;
                if *position == transom_core::TabPosition::Left {
                    self.state.sidebar.visible = true;
                }
                cx.notify();
            }
            SettingsEvent::ApplyTriggers(triggers) => {
                let terminals: Vec<Entity<TerminalView>> = self
                    .state
                    .workspace
                    .tabs
                    .iter()
                    .filter_map(|tab| tab.terminal())
                    .collect();
                for terminal in terminals {
                    terminal.update(cx, |view, _| view.reload_triggers(triggers));
                }
            }
            SettingsEvent::OpenShortcutCapture(id) => {
                self.state
                    .window_requests
                    .push_settings(SettingsRequest::OpenShortcutCapture(id));
            }
            SettingsEvent::OpenBackupImport => {
                self.state
                    .window_requests
                    .push_settings(SettingsRequest::OpenBackupImport);
            }
            SettingsEvent::OpenBackupExport => {
                self.state
                    .window_requests
                    .push_settings(SettingsRequest::OpenBackupExport);
            }
            SettingsEvent::OpenAuditViewer => {
                self.state
                    .window_requests
                    .push_settings(SettingsRequest::OpenAuditViewer);
            }
            SettingsEvent::CloudSyncPush => {
                self.state
                    .window_requests
                    .push_settings(SettingsRequest::CloudSyncPush);
            }
            SettingsEvent::CloudSyncPull => {
                self.state
                    .window_requests
                    .push_settings(SettingsRequest::CloudSyncPull);
            }
            SettingsEvent::GitSyncNow => {
                self.state
                    .window_requests
                    .push_settings(SettingsRequest::GitSyncNow);
            }
            SettingsEvent::Status(message) => {
                self.state.status = Some(message.clone());
                cx.notify();
            }
            SettingsEvent::Error(message) => {
                self.state.set_error(message.clone());
                cx.notify();
            }
        }
    }

    /// 执行一条设置实体排队的窗口动作。
    pub(crate) fn handle_settings_request(
        &mut self,
        request: super::SettingsRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match request {
            SettingsRequest::OpenShortcutCapture(id) => {
                self.open_shortcut_capture(id, window, cx);
            }
            SettingsRequest::OpenBackupImport => self.open_backup_import(window, cx),
            SettingsRequest::OpenBackupExport => self.open_backup_export(window, cx),
            SettingsRequest::OpenAuditViewer => self.open_audit_viewer(None, window, cx),
            SettingsRequest::CloudSyncPush => self.start_cloud_sync_push(window, cx),
            SettingsRequest::CloudSyncPull => self.start_cloud_sync_pull(window, cx),
            SettingsRequest::GitSyncNow => self.start_git_sync(window, cx),
        }
    }
}
