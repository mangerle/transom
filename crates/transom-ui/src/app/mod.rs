//! GPUI 顶层应用外壳状态与生命周期主入口。

mod backup_prompt;
mod bootstrap;
mod completion_source;
mod container_requests;
pub(crate) mod event_handlers;
mod event_terminal_ready;
pub(crate) mod events;
mod frame_effects;
mod language;
mod modal_hub;
mod overlays;
mod persistence;
mod render;
mod runtime;
mod settings_global;

pub(crate) use settings_global::{
    app_settings, app_settings_mut, install_app_settings, update_app_settings,
};
mod secret_prefill;
mod session_duplicate;
mod session_ops;
mod sftp_terminal;
mod state;
pub(crate) mod tab;
mod tab_coordination;
mod task_hub;
mod types;
pub(crate) mod update;
mod window_requests;

pub(crate) use completion_source::CompletionSource;
pub(crate) use modal_hub::{GitModals, ModalHub, ToolModalEntities};
pub(crate) use persistence::Persistence;
pub(crate) use runtime::spawn_blocking_local;
pub(crate) use secret_prefill::prefill_secret_fields;
pub(crate) use state::AppState;
pub(crate) use tab::{
    SessionMenuData, TabCloseAction, TabContent, TabGroup, TabGroupOwner, TabProgress, TabStatus,
    WorkspaceTab,
};
pub(crate) use task_hub::TaskHub;

use crate::UiServices;
use crate::broadcast::BroadcastBarState;
use crate::editor::EditorPanels;
use crate::keymap;
use crate::keymap::ShortcutRecordingState;
use crate::palette::localized_error;
use crate::sidebar::SidebarState;
use crate::snippets::SnippetPanelState;
use crate::workspace::WorkspaceState;
use gpui_kit::component::WindowExt;
use gpui_kit::component::command::CommandState;
use gpui_kit::component::input::{InputEvent, InputState, TextareaState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::resizable::ResizableState;
use gpui_kit::*;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use transom_core::{
    AppCommand, AppSettings, GitCommand, SessionCatalog, StartupTabs, TabId, TabPosition,
    UiThemeMode,
};

pub(crate) use types::*;
pub(crate) use window_requests::{WindowRequest, WindowRequests};

/// 命令发送失败的界面文案：队列满提示稍后重试，控制器退出提示重启应用。
pub(crate) fn command_send_error_text(error: flume::TrySendError<AppCommand>) -> String {
    match error {
        flume::TrySendError::Full(_) => rust_i18n::t!("error.command_queue_full").to_string(),
        flume::TrySendError::Disconnected(_) => {
            rust_i18n::t!("error.controller_stopped").to_string()
        }
    }
}

pub(crate) struct AppShell {
    /// 全部应用状态集中存放；外壳只负责渲染、事件路由与生命周期。
    pub(crate) state: AppState,
}

/// 提示通知的类型标记：只用于给 `Notification::id1` 与 `remove_notification1`
/// 配对，让定时器精确关闭自己那一条，不参与任何业务逻辑。
struct ToastMarker;

impl AppShell {
    /// 右上角操作结果提示，3 秒后自动收起。
    ///
    /// 设计原理（Why）：上游通知组件的自动隐藏时长写死 5 秒，不接受这个与
    /// 需求不符的时长，改成 `autohide(false)` + 显式定时器关闭，保证看到的
    /// 与实际发生的一致。定时器只有关闭自己这一步，句柄显式 `detach`，
    /// 不给每条提示再养一个字段。
    pub(crate) fn show_toast(
        &mut self,
        message: impl Into<String>,
        success: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let msg = message.into();
        if self.state.ui.feedback.should_dedup_toast(&msg) {
            return;
        }
        let notification = if success {
            Notification::success(msg)
        } else {
            Notification::error(msg)
        };
        self.push_toast(notification, window, cx);
    }

    /// 右上角中性提示，用于正在执行、传输开始等非成功/失败回执。
    pub(crate) fn show_info_toast(
        &mut self,
        message: impl Into<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let msg = message.into();
        if self.state.ui.feedback.should_dedup_toast(&msg) {
            return;
        }
        self.push_toast(Notification::info(msg), window, cx);
    }

    fn push_toast(
        &mut self,
        notification: Notification,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state.ui.feedback.toast_seq = self.state.ui.feedback.toast_seq.wrapping_add(1);
        let sequence = self.state.ui.feedback.toast_seq;
        let key = format!("transom-toast-{sequence}");
        window.push_notification(
            notification.id1::<ToastMarker>(key.clone()).autohide(false),
            cx,
        );
        let task = cx.spawn_in(window, async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_secs(3))
                .await;
            let _ = this.update_in(cx, |_, window, cx| {
                window.remove_notification1::<ToastMarker>(key, cx);
            });
        });
        if self.state.runtime.tasks.background.toast_tasks.len() >= 32 {
            self.state.runtime.tasks.background.toast_tasks.pop_front();
        }
        self.state
            .runtime
            .tasks
            .background
            .toast_tasks
            .push_back((sequence, task));
    }

    /// 向命令通道投递一条应用命令。
    pub(crate) fn send_command(&mut self, command: impl Into<AppCommand>) -> bool {
        self.state.send_command(command)
    }

    /// 设置会在固定时间后清除的底栏操作完成提示。
    pub(crate) fn set_transient_status(
        &mut self,
        message: impl Into<String>,
        cx: &mut Context<Self>,
    ) {
        self.state.set_transient_status(message, cx);
    }

    /// 打开偏好设置弹窗；`page` 为 `Some` 时直接定位到指定页。
    ///
    /// 回填与预览快照都归设置实体，外壳只负责把目录快照推进去并请求打开；
    /// 页号越界时由实体退化为普通打开。
    pub(crate) fn open_settings_modal(
        &mut self,
        page: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = self.state.ui.modals.settings.clone();
        let catalog = self.state.session.catalog.clone();
        let interval = self.state.features.monitor.read(cx).interval_mode();
        view.update(cx, |view, cx| {
            view.set_catalog(catalog);
            view.set_monitor_interval(interval);
            view.prepare_open(window, page, cx);
        });
        cx.notify();
    }

    /// 按启动档位恢复工作台。
    fn restore_startup_tabs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match app_settings(cx).window.startup_tabs {
            StartupTabs::RestoreLast => {
                let saved_groups = app_settings(cx).window.previous_tab_groups.clone();
                let saved_tilings = app_settings(cx).window.tab_tilings.clone();
                if !saved_groups.is_empty() {
                    let mut restored = Vec::with_capacity(saved_groups.len());
                    for group in &saved_groups {
                        restored.push(self.restore_tab_group(group, window, cx));
                    }
                    let mut focus = None;
                    for saved in &saved_tilings {
                        if let Some(group) = self.state.restore_tab_tiling(saved, &restored) {
                            focus.get_or_insert(group);
                        }
                    }
                    if let Some(group) = focus
                        && let Some(index) = self.state.workspace.group_index(group)
                    {
                        let pane = self.state.workspace.groups[index].active_pane;
                        self.activate_tab(pane, window, cx);
                    }
                } else if !app_settings(cx).window.previous_tab_sessions.is_empty() {
                    let session_keys = app_settings(cx).window.previous_tab_sessions.clone();
                    for key in session_keys {
                        if key == "local" {
                            crate::connection::open_local_terminal(&mut self.state, window, cx);
                        } else if let Some(session) = tab_coordination::saved_session_for_key(
                            &self.state.session.catalog,
                            &key,
                        ) {
                            crate::connection::connect_session(
                                &mut self.state,
                                session.id,
                                window,
                                cx,
                            );
                        }
                    }
                }
            }
            StartupTabs::EmptyWorkspace => {}
            StartupTabs::NewLocalTerminal => {
                crate::connection::open_local_terminal(&mut self.state, window, cx)
            }
        }
    }

    /// 恢复上次打开的 API 项目标签；项目不存在或 id 无效时跳过。
    fn restore_api_tabs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let project_ids = app_settings(cx).window.previous_api_projects.clone();
        for project_id in project_ids {
            let Ok(project_id) = project_id.parse::<transom_api::ApiProjectId>() else {
                log::warn!("恢复 API 标签时项目 id 无效，已跳过: {project_id}");
                continue;
            };
            let project = self
                .state
                .features
                .api_sidebar
                .read(cx)
                .state
                .registry
                .project(project_id)
                .cloned();
            if let Some(project) = project {
                crate::api_client::open_api_project(&mut self.state, project, window, cx);
            } else {
                log::warn!("恢复 API 标签时项目不存在，已跳过: {project_id}");
            }
        }
    }

    /// 每 24 小时对已连接的容器工作区执行一次轻量清理；默认关闭。
    fn tick_container_cleanup(&mut self, cx: &mut Context<Self>) {
        const INTERVAL: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);
        if !app_settings(cx).container_automation.auto_prune_enabled {
            self.state.runtime.automation.container_auto_prune_last = None;
            return;
        }
        let now = std::time::Instant::now();
        let Some(last) = self.state.runtime.automation.container_auto_prune_last else {
            // 首次启用只开始计时，不立即执行破坏性清理。
            self.state.runtime.automation.container_auto_prune_last = Some(now);
            return;
        };
        if now.duration_since(last) < INTERVAL {
            return;
        }
        let tabs: Vec<TabId> = self
            .state
            .workspace
            .tabs
            .iter()
            .filter_map(|tab| {
                let workspace = tab.container_workspace()?;
                (workspace.read(cx).state.status
                    == crate::container::ContainerWorkspaceStatus::Connected)
                    .then_some(tab.id)
            })
            .collect();
        self.state.runtime.automation.container_auto_prune_last = Some(now);
        for tab_id in tabs {
            let workspace = self.state.container_workspace(tab_id);
            if let Some(workspace) = workspace {
                workspace.update(cx, |workspace, _| workspace.request_auto_prune());
            }
        }
    }

    /// 每分钟节拍：达到配置间隔时触发一次后台 Git 同步。
    fn tick_git_sync(&mut self, cx: &mut Context<Self>) {
        let interval = app_settings(cx)
            .sync
            .git
            .as_ref()
            .map_or(0, |git| git.interval_minutes);
        if self
            .state
            .runtime
            .automation
            .git_sync_schedule
            .tick(interval)
        {
            crate::cloud_sync::CloudSyncActions::new(&mut self.state).start_git_sync_scheduled(cx);
        }
    }

    /// 每分钟节拍：达到 Git 自动抓取配置间隔时，对所有打开的 Git 项目触发一次 git fetch。
    fn tick_git_auto_fetch(&mut self, cx: &mut Context<Self>) {
        let interval = app_settings(cx).git.auto_fetch_interval as u64;
        if interval == 0 {
            return;
        }
        let (due, projects) = self.state.update_git_registry(cx, |git| {
            let due = git.auto_fetch_schedule.tick(interval);
            (
                due,
                if due {
                    git.projects.clone()
                } else {
                    Vec::new()
                },
            )
        });
        if due {
            for project in projects {
                self.send_command(GitCommand::Fetch { project });
            }
        }
    }
}
