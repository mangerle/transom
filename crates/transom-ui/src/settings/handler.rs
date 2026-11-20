//! 设置实体事件的跨域处理与渲染期请求消费。
//!
//! 设置实体只改自己领域内的状态；活窗格外观、SFTP 列宽、监控采样、云同步与
//! 文件选择器都属于窗口级动作，集中在这里由外壳执行。处理函数运行在实体
//! 事件派发阶段，**不得回读设置实体**：事件已自带快照，需要窗口的动作排进
//! `window_requests`，由 [`AppShell::flush_window_requests`]
//! 在渲染期执行。

use super::{SettingsEvent, SettingsRequest};
use crate::app::{AppShell, AppState};
use crate::sidebar::SidebarSection;
use gpui_kit::*;
use transom_terminal::TerminalView;

/// 设置实体发出的跨域意图。
pub(crate) fn on_settings_event(
    app_state: &mut AppState,
    event: &SettingsEvent,
    cx: &mut Context<AppShell>,
) {
    match event {
        SettingsEvent::Saved(settings) => on_settings_saved(app_state, settings, cx),
        SettingsEvent::Cancelled | SettingsEvent::ProfilePreview => {
            crate::settings::apply_appearance_to_live_panes(app_state, cx);
            cx.notify();
        }
        SettingsEvent::CatalogChanged(catalog) => {
            app_state.session.catalog = (**catalog).clone();
            app_state.refresh_tab_sessions();
            app_state.persist_catalog();
            cx.notify();
        }
        SettingsEvent::SetSftpPanelSide(side) => {
            app_state.set_sftp_panel_side(*side, cx);
        }
        SettingsEvent::MonitorIntervalChanged(mode) => {
            crate::monitor::MonitorOps::new(app_state).set_monitor_interval_mode(*mode, cx);
        }
        SettingsEvent::MonitorSectionsChanged(collapse) => {
            app_state
                .features
                .monitor
                .update(cx, |view, _| view.state.collapse = *collapse);
            cx.notify();
        }
        SettingsEvent::MonitorDrawerWidthChanged(width) => {
            app_state
                .features
                .monitor
                .update(cx, |view, _| view.set_drawer_width(*width));
            cx.notify();
        }
        SettingsEvent::PerformanceHudChanged(enabled) => {
            crate::performance::set_enabled(*enabled, cx);
            cx.notify();
        }
        SettingsEvent::TabPositionChanged(position) => {
            if *position == transom_core::TabPosition::Left {
                let section = match app_state.session.sidebar.section() {
                    SidebarSection::Sessions => SidebarSection::Tabs,
                    section => section,
                };
                app_state.session.sidebar.show_section(section);
            } else if app_state.session.sidebar.section() == SidebarSection::Tabs {
                app_state
                    .session
                    .sidebar
                    .select_section(SidebarSection::Sessions);
            }
            cx.notify();
        }
        SettingsEvent::ApplyTriggers(triggers) => {
            apply_triggers_to_terminals(app_state, triggers.as_ref(), cx);
        }
        SettingsEvent::OpenShortcutCapture(id) => {
            queue_settings_request(app_state, SettingsRequest::OpenShortcutCapture(id));
        }
        SettingsEvent::OpenBackupImport => {
            queue_settings_request(app_state, SettingsRequest::OpenBackupImport);
        }
        SettingsEvent::OpenBackupExport => {
            queue_settings_request(app_state, SettingsRequest::OpenBackupExport);
        }
        SettingsEvent::OpenAuditViewer => {
            queue_settings_request(app_state, SettingsRequest::OpenAuditViewer);
        }
        SettingsEvent::CloudSyncCheck => {
            queue_settings_request(app_state, SettingsRequest::CloudSyncCheck);
        }
        SettingsEvent::CloudSyncTest => {
            queue_settings_request(app_state, SettingsRequest::CloudSyncTest);
        }
        SettingsEvent::CloudSyncPush => {
            queue_settings_request(app_state, SettingsRequest::CloudSyncPush);
        }
        SettingsEvent::CloudSyncPull => {
            queue_settings_request(app_state, SettingsRequest::CloudSyncPull);
        }
        SettingsEvent::GitSyncNow => {
            queue_settings_request(app_state, SettingsRequest::GitSyncNow);
        }
        SettingsEvent::Status(message) => {
            app_state.ui.status = Some(message.clone());
            cx.notify();
        }
        SettingsEvent::Error(message) => {
            app_state.set_error(message.clone());
            cx.notify();
        }
        SettingsEvent::CheckForUpdates => {
            let update = app_state.features.update.clone();
            update.update(cx, |view, cx| view.open_and_check(cx));
            cx.notify();
        }
    }
}

/// 设置保存后：应用外观、快捷键、块视图与触发器快照。
fn on_settings_saved(
    app_state: &mut AppState,
    settings: &transom_core::AppSettings,
    cx: &mut Context<AppShell>,
) {
    crate::performance::set_enabled(settings.performance.show_fps_hud, cx);
    crate::settings::apply_appearance_to_live_panes(app_state, cx);
    crate::shortcuts::apply_shortcut_settings(app_state, cx);
    crate::settings::apply_block_view_settings(app_state, cx);
    apply_triggers_to_terminals(app_state, &settings.triggers, cx);
    // Git 可执行文件与提交作者是进程级配置：界面只投递意图，写入权归控制平面。
    app_state.send_command(transom_core::GitCommand::Configure {
        program: settings.git.executable_path.clone(),
        author_name: settings.git.author_name.clone(),
        author_email: settings.git.author_email.clone(),
    });
    cx.notify();
}

/// 把触发器快照推给所有已打开终端。
fn apply_triggers_to_terminals(
    app_state: &mut AppState,
    triggers: &transom_core::TriggerSettings,
    cx: &mut Context<AppShell>,
) {
    let terminals: Vec<Entity<TerminalView>> = app_state
        .workspace
        .tabs
        .iter()
        .filter_map(|tab| tab.terminal())
        .collect();
    for terminal in terminals {
        terminal.update(cx, |view, _| view.reload_triggers(triggers));
    }
}

/// 设置实体要求外壳做的窗口动作统一入队。
fn queue_settings_request(app_state: &mut AppState, request: SettingsRequest) {
    app_state.ui.window_requests.push_settings(request);
}

/// 执行一条设置实体排队的窗口动作。
pub(crate) fn handle_settings_request(
    app_state: &mut AppState,
    request: super::SettingsRequest,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    match request {
        SettingsRequest::OpenShortcutCapture(id) => {
            crate::shortcuts::open_shortcut_capture(app_state, id, window, cx);
        }
        SettingsRequest::OpenBackupImport => app_state.open_backup_import(window, cx),
        SettingsRequest::OpenBackupExport => app_state.open_backup_export(window, cx),
        SettingsRequest::OpenAuditViewer => {
            crate::audit_viewer::open_audit_viewer(app_state, None, window, cx)
        }
        SettingsRequest::CloudSyncCheck => {
            crate::cloud_sync::CloudSyncActions::new(app_state).start_cloud_sync_check(window, cx)
        }
        SettingsRequest::CloudSyncTest => {
            crate::cloud_sync::CloudSyncActions::new(app_state).start_cloud_sync_test(window, cx)
        }
        SettingsRequest::CloudSyncPush => {
            crate::cloud_sync::CloudSyncActions::new(app_state).start_cloud_sync_push(window, cx)
        }
        SettingsRequest::CloudSyncPull => {
            crate::cloud_sync::CloudSyncActions::new(app_state).start_cloud_sync_pull(window, cx)
        }
        SettingsRequest::GitSyncNow => {
            crate::cloud_sync::CloudSyncActions::new(app_state).start_git_sync(window, cx)
        }
    }
}
