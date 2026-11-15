//! 设置实体事件的跨域处理与渲染期请求消费。
//!
//! 设置实体只改自己领域内的状态；活窗格外观、SFTP 列宽、监控采样、云同步与
//! 文件选择器都属于窗口级动作，集中在这里由外壳执行。处理函数运行在实体
//! 事件派发阶段，**不得回读设置实体**：事件已自带快照，需要窗口的动作排进
//! `window_requests`，由 [`AppShell::flush_window_requests`]
//! 在渲染期执行。

use super::{SettingsEvent, SettingsRequest};
use crate::app::AppShell;
use crate::sidebar::SidebarSection;
use gpui_kit::*;
use transom_terminal::TerminalView;

/// 设置实体发出的跨域意图。
pub(crate) fn on_settings_event(
    shell: &mut AppShell,
    event: &SettingsEvent,
    cx: &mut Context<AppShell>,
) {
    match event {
        SettingsEvent::Saved(settings) => on_settings_saved(shell, settings, cx),
        SettingsEvent::Cancelled | SettingsEvent::ProfilePreview => {
            crate::settings::apply_appearance_to_live_panes(shell, cx);
            cx.notify();
        }
        SettingsEvent::CatalogChanged(catalog) => {
            shell.state.catalog = (**catalog).clone();
            shell.state.refresh_tab_sessions();
            shell.state.persist_catalog();
            cx.notify();
        }
        SettingsEvent::SetSftpPanelSide(side) => {
            shell.set_sftp_panel_side(*side, cx);
        }
        SettingsEvent::MonitorIntervalChanged(mode) => {
            crate::monitor::set_monitor_interval_mode(shell, *mode, cx);
        }
        SettingsEvent::MonitorSectionsChanged(collapse) => {
            shell
                .state
                .monitor
                .update(cx, |view, _| view.state.collapse = *collapse);
            cx.notify();
        }
        SettingsEvent::MonitorDrawerWidthChanged(width) => {
            shell
                .state
                .monitor
                .update(cx, |view, _| view.set_drawer_width(*width));
            cx.notify();
        }
        SettingsEvent::TabPositionChanged(position) => {
            if *position == transom_core::TabPosition::Left {
                let section = match shell.state.sidebar.section() {
                    SidebarSection::Sessions => SidebarSection::Tabs,
                    section => section,
                };
                shell.state.sidebar.show_section(section);
            } else if shell.state.sidebar.section() == SidebarSection::Tabs {
                shell.state.sidebar.select_section(SidebarSection::Sessions);
            }
            cx.notify();
        }
        SettingsEvent::ApplyTriggers(triggers) => {
            apply_triggers_to_terminals(shell, triggers.as_ref(), cx);
        }
        SettingsEvent::OpenShortcutCapture(id) => {
            queue_settings_request(shell, SettingsRequest::OpenShortcutCapture(id));
        }
        SettingsEvent::OpenBackupImport => {
            queue_settings_request(shell, SettingsRequest::OpenBackupImport);
        }
        SettingsEvent::OpenBackupExport => {
            queue_settings_request(shell, SettingsRequest::OpenBackupExport);
        }
        SettingsEvent::OpenAuditViewer => {
            queue_settings_request(shell, SettingsRequest::OpenAuditViewer);
        }
        SettingsEvent::CloudSyncCheck => {
            queue_settings_request(shell, SettingsRequest::CloudSyncCheck);
        }
        SettingsEvent::CloudSyncTest => {
            queue_settings_request(shell, SettingsRequest::CloudSyncTest);
        }
        SettingsEvent::CloudSyncPush => {
            queue_settings_request(shell, SettingsRequest::CloudSyncPush);
        }
        SettingsEvent::CloudSyncPull => {
            queue_settings_request(shell, SettingsRequest::CloudSyncPull);
        }
        SettingsEvent::GitSyncNow => {
            queue_settings_request(shell, SettingsRequest::GitSyncNow);
        }
        SettingsEvent::Status(message) => {
            shell.state.status = Some(message.clone());
            cx.notify();
        }
        SettingsEvent::Error(message) => {
            shell.state.set_error(message.clone());
            cx.notify();
        }
        SettingsEvent::RestartForUpdate => {
            // 这里只置位并退出事件循环；新进程必须等 `main` 完成控制器
            // 优雅停机后再拉起，避免 SSH 连接与审计写入被直接截断。
            shell
                .state
                .restart_requested
                .store(true, std::sync::atomic::Ordering::Relaxed);
            cx.quit();
        }
    }
}

/// 设置保存后：应用外观、快捷键、块视图与触发器快照。
fn on_settings_saved(
    shell: &mut AppShell,
    settings: &transom_core::AppSettings,
    cx: &mut Context<AppShell>,
) {
    crate::settings::apply_appearance_to_live_panes(shell, cx);
    crate::shortcuts::apply_shortcut_settings(shell, cx);
    crate::settings::apply_block_view_settings(shell, cx);
    apply_triggers_to_terminals(shell, &settings.triggers, cx);
    // Git 可执行文件与提交作者是进程级配置：界面只投递意图，写入权归控制平面。
    shell.send_command(transom_core::GitCommand::Configure {
        program: settings.git.executable_path.clone(),
        author_name: settings.git.author_name.clone(),
        author_email: settings.git.author_email.clone(),
    });
    cx.notify();
}

/// 把触发器快照推给所有已打开终端。
fn apply_triggers_to_terminals(
    shell: &mut AppShell,
    triggers: &transom_core::TriggerSettings,
    cx: &mut Context<AppShell>,
) {
    let terminals: Vec<Entity<TerminalView>> = shell
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

/// 设置实体要求外壳做的窗口动作统一入队。
fn queue_settings_request(shell: &mut AppShell, request: SettingsRequest) {
    shell.state.window_requests.push_settings(request);
}

/// 执行一条设置实体排队的窗口动作。
pub(crate) fn handle_settings_request(
    shell: &mut AppShell,
    request: super::SettingsRequest,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    match request {
        SettingsRequest::OpenShortcutCapture(id) => {
            crate::shortcuts::open_shortcut_capture(shell, id, window, cx);
        }
        SettingsRequest::OpenBackupImport => shell.open_backup_import(window, cx),
        SettingsRequest::OpenBackupExport => shell.open_backup_export(window, cx),
        SettingsRequest::OpenAuditViewer => {
            crate::audit_viewer::open_audit_viewer(shell, None, window, cx)
        }
        SettingsRequest::CloudSyncCheck => {
            crate::cloud_sync::start_cloud_sync_check(shell, window, cx)
        }
        SettingsRequest::CloudSyncTest => {
            crate::cloud_sync::start_cloud_sync_test(shell, window, cx)
        }
        SettingsRequest::CloudSyncPush => {
            crate::cloud_sync::start_cloud_sync_push(shell, window, cx)
        }
        SettingsRequest::CloudSyncPull => {
            crate::cloud_sync::start_cloud_sync_pull(shell, window, cx)
        }
        SettingsRequest::GitSyncNow => crate::cloud_sync::start_git_sync(shell, window, cx),
    }
}
