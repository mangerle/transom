//! 云同步实体事件桥接：消费来自 [`super::CloudSyncView`] 的跨领域意图。

use super::entity::{CloudSyncEvent, CloudSyncSidebarAction};
use crate::app::AppShell;
use crate::sidebar::SidebarSection;
use gpui_kit::*;

/// 显示云同步侧边栏；活动栏点击统一走这里，保持收起再展开行为一致。
pub(crate) fn toggle_cloud_sync_sidebar(shell: &mut AppShell, cx: &mut Context<AppShell>) {
    shell
        .state
        .sidebar
        .toggle_section(SidebarSection::CloudSync);
    cx.notify();
}

/// 处理云同步侧栏实体上报的意图事件。
pub(crate) fn on_cloud_sync_event(
    shell: &mut AppShell,
    event: &CloudSyncEvent,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    match event {
        CloudSyncEvent::OpenSettings => {
            shell.open_settings_modal(None, window, cx);
        }
        CloudSyncEvent::TriggerAction(action) => {
            let mut actions = crate::cloud_sync::CloudSyncActions::new(&mut shell.state);
            match action {
                CloudSyncSidebarAction::GitSyncNow => actions.start_git_sync(window, cx),
                CloudSyncSidebarAction::Check => actions.start_cloud_sync_check(window, cx),
                CloudSyncSidebarAction::Test => actions.start_cloud_sync_test(window, cx),
                CloudSyncSidebarAction::Push => actions.start_cloud_sync_push(window, cx),
                CloudSyncSidebarAction::Pull => actions.start_cloud_sync_pull(window, cx),
                CloudSyncSidebarAction::ForcePull => actions.start_cloud_sync_force_pull(cx),
                CloudSyncSidebarAction::ForcePush => actions.start_cloud_sync_force_push(cx),
            }
        }
    }
}
