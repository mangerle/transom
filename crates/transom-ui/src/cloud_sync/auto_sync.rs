//! WebDAV/S3 自动同步调度：启动检查、周期检查与本地变化防抖。

use super::common::*;
use super::scheduler::CloudSyncTick;
use crate::app::app_settings;
use transom_core::SyncTrigger;

impl AppShell {
    /// 每秒推进一次云同步节拍；真正的网络动作仍通过控制器单飞执行。
    pub(crate) fn tick_cloud_sync_auto(&mut self, cx: &mut Context<Self>) {
        if self.state.cloud_sync.is_running() || self.state.tasks.background.sync_pull.is_some() {
            return;
        }
        let settings = app_settings(cx).sync.clone();
        if !settings.endpoint_enabled() {
            self.state.cloud_sync.scheduler.reset();
            return;
        }
        let action = self.state.cloud_sync.scheduler.tick(
            std::time::Instant::now(),
            settings.enabled,
            settings.auto_check_on_startup,
            settings.auto_push_on_change,
            settings.auto_push_debounce_seconds,
            self.state.catalog_revision,
        );
        let trigger = match action {
            Some(CloudSyncTick::StartupCheck) => SyncTrigger::Startup,
            Some(CloudSyncTick::PeriodicCheck) => SyncTrigger::Periodic,
            Some(CloudSyncTick::LocalChange) => SyncTrigger::LocalChange,
            None => return,
        };
        self.dispatch_cloud_sync_check(settings, trigger, cx);
    }
}
