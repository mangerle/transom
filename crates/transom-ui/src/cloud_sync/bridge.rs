//! 云同步实体事件桥接：把侧栏意图转换为窄上下文动作或外壳请求。

use super::CloudSyncActions;
use crate::app::{AppShell, AppState};
use gpui_kit::*;

/// 云同步事件需要外壳执行的窗口级动作。
pub(crate) enum CloudSyncRequest {
    OpenSettings,
}

/// 显示云同步侧边栏。
pub(crate) fn toggle_cloud_sync_sidebar(state: &mut AppState, cx: &mut Context<AppShell>) {
    state
        .session
        .sidebar
        .toggle_section(crate::sidebar::SidebarSection::CloudSync);
    cx.notify();
}

/// 处理云同步侧栏实体上报的意图事件。
pub(crate) fn on_cloud_sync_event(
    state: &mut AppState,
    event: &super::CloudSyncEvent,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) -> Option<CloudSyncRequest> {
    use super::CloudSyncEvent;
    use super::entity::CloudSyncSidebarAction;
    match event {
        CloudSyncEvent::OpenSettings => Some(CloudSyncRequest::OpenSettings),
        CloudSyncEvent::TriggerAction(action) => {
            let mut actions = CloudSyncActions::new(state);
            match action {
                CloudSyncSidebarAction::GitSyncNow => actions.start_git_sync(window, cx),
                CloudSyncSidebarAction::Check => actions.start_cloud_sync_check(window, cx),
                CloudSyncSidebarAction::Test => actions.start_cloud_sync_test(window, cx),
                CloudSyncSidebarAction::Push => actions.start_cloud_sync_push(window, cx),
                CloudSyncSidebarAction::Pull => actions.start_cloud_sync_pull(window, cx),
                CloudSyncSidebarAction::ForcePull => actions.start_cloud_sync_force_pull(cx),
                CloudSyncSidebarAction::ForcePush => actions.start_cloud_sync_force_push(cx),
            }
            None
        }
    }
}
