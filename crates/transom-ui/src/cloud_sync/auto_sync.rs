//! WebDAV/S3 自动同步调度：启动检查、周期检查与本地变化防抖。

use super::common::*;
use super::scheduler::CloudSyncTick;
use crate::app::app_settings;
use transom_core::SyncTrigger;

/// 每秒推进一次云同步节拍；真正的网络动作仍通过控制器单飞执行。
pub(crate) fn tick_cloud_sync_auto(shell: &mut AppShell, cx: &mut Context<AppShell>) {
    if shell.state.cloud_sync.is_running() || shell.state.tasks.background.sync_pull.is_some() {
        return;
    }
    let settings = app_settings(cx).sync.clone();
    if !settings.endpoint_enabled() {
        shell.state.cloud_sync.scheduler.reset();
        return;
    }
    let action = shell.state.cloud_sync.scheduler.tick(
        std::time::Instant::now(),
        settings.enabled,
        settings.auto_check_on_startup,
        settings.auto_push_on_change,
        settings.auto_push_debounce_seconds,
        shell.state.catalog_revision,
    );
    let trigger = match action {
        Some(CloudSyncTick::StartupCheck) => SyncTrigger::Startup,
        Some(CloudSyncTick::PeriodicCheck) => SyncTrigger::Periodic,
        Some(CloudSyncTick::LocalChange) => SyncTrigger::LocalChange,
        None => return,
    };
    crate::cloud_sync::dispatch_cloud_sync_check(shell, settings, trigger, cx);
}
