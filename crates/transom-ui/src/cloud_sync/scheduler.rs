//! Git 与 WebDAV/S3 自动同步节拍器。

use std::time::{Duration, Instant};

/// 定时 Git 同步的节拍器：按分钟累计，达到配置间隔才触发一次。
///
/// 独立于任务容器：它记录的是调度进度而非任务句柄，放进 `TaskHub` 会让
/// “纳管后台任务生命周期”的职责失真。
#[derive(Debug, Default)]
pub(crate) struct GitSyncScheduler {
    elapsed_minutes: u64,
}

impl GitSyncScheduler {
    /// 推进一分钟；到达配置间隔返回 `true` 并清零。
    ///
    /// `interval_minutes` 为 0 表示关闭定时同步，此时同时清零累计值，
    /// 避免关闭再开启后立即触发一次。
    pub(crate) fn tick(&mut self, interval_minutes: u64) -> bool {
        if interval_minutes == 0 {
            self.elapsed_minutes = 0;
            return false;
        }
        self.elapsed_minutes = self.elapsed_minutes.saturating_add(1);
        if self.elapsed_minutes >= interval_minutes {
            self.elapsed_minutes = 0;
            return true;
        }
        false
    }
}

/// WebDAV/S3 自动同步的一次待执行动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CloudSyncTick {
    StartupCheck,
    PeriodicCheck,
    LocalChange,
}

/// WebDAV/S3 自动同步节拍器。
///
/// 设计原理（Why）：启动检查与周期检查都需要远端只读探测；本地变化则先等待
/// 防抖窗口，避免连续编辑时反复上传。节拍器只产生意图，真正的单飞、冲突判定
/// 和 UI 状态仍由外壳与控制器负责。
#[derive(Debug, Default)]
pub(crate) struct CloudSyncScheduler {
    initialized: bool,
    startup_pending: bool,
    last_check_at: Option<Instant>,
    observed_revision: Option<u64>,
    changed_at: Option<Instant>,
}

impl CloudSyncScheduler {
    /// 周期检查间隔：即使关闭启动检查，也会按该间隔探测远端。
    const PERIODIC_CHECK_INTERVAL: Duration = Duration::from_secs(15 * 60);

    /// 推进调度；一次调用最多返回一个动作。
    pub(crate) fn tick(
        &mut self,
        now: Instant,
        enabled: bool,
        auto_check_on_startup: bool,
        auto_push_on_change: bool,
        debounce_seconds: u64,
        catalog_revision: u64,
    ) -> Option<CloudSyncTick> {
        if !enabled {
            self.reset();
            return None;
        }
        if !self.initialized {
            self.initialized = true;
            self.startup_pending = auto_check_on_startup;
            self.last_check_at = Some(now);
            self.observed_revision = Some(catalog_revision);
        } else if self.observed_revision != Some(catalog_revision) {
            self.observed_revision = Some(catalog_revision);
            if auto_push_on_change {
                self.changed_at = Some(now);
            }
        }

        if auto_push_on_change
            && let Some(changed_at) = self.changed_at
            && now.duration_since(changed_at) >= Duration::from_secs(debounce_seconds)
        {
            self.changed_at = None;
            return Some(CloudSyncTick::LocalChange);
        }
        if self.startup_pending {
            self.startup_pending = false;
            self.last_check_at = Some(now);
            return Some(CloudSyncTick::StartupCheck);
        }
        if self
            .last_check_at
            .is_none_or(|last| now.duration_since(last) >= Self::PERIODIC_CHECK_INTERVAL)
        {
            self.last_check_at = Some(now);
            return Some(CloudSyncTick::PeriodicCheck);
        }
        None
    }

    /// 记录一次检查完成，周期计时从完成时刻重新开始。
    pub(crate) fn mark_check_finished(&mut self, now: Instant) {
        self.last_check_at = Some(now);
    }

    /// 远端结果刚写回本机时同步观察到的版本，避免把同步自身的落盘误判为本地编辑。
    pub(crate) fn mark_catalog_synced(&mut self, catalog_revision: u64) {
        self.observed_revision = Some(catalog_revision);
        self.changed_at = None;
    }

    /// 设置变化或暂停后重置全部节拍，重新启用时从头计时。
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::{CloudSyncScheduler, CloudSyncTick, GitSyncScheduler};
    use std::time::{Duration, Instant};

    /// 间隔到达才触发，触发后重新计时。
    #[test]
    fn git_scheduler_fires_at_interval_and_resets() {
        let mut scheduler = GitSyncScheduler::default();
        assert!(!scheduler.tick(3));
        assert!(!scheduler.tick(3));
        assert!(scheduler.tick(3));
        assert!(!scheduler.tick(3));
    }

    /// 关闭定时同步（间隔 0）会清空累计，重新开启后不会立即触发。
    #[test]
    fn zero_interval_disables_and_resets_progress() {
        let mut scheduler = GitSyncScheduler::default();
        assert!(!scheduler.tick(2));
        assert!(!scheduler.tick(0));
        assert!(!scheduler.tick(2));
    }

    #[test]
    fn startup_check_fires_once_then_periodic_fires_later() {
        let now = Instant::now();
        let mut scheduler = CloudSyncScheduler::default();
        assert_eq!(
            scheduler.tick(now, true, true, false, 60, 0),
            Some(CloudSyncTick::StartupCheck)
        );
        assert_eq!(scheduler.tick(now, true, true, false, 60, 0), None);
        assert_eq!(
            scheduler.tick(now + Duration::from_secs(15 * 60), true, true, false, 60, 0),
            Some(CloudSyncTick::PeriodicCheck)
        );
    }

    #[test]
    fn local_change_waits_for_debounce() {
        let now = Instant::now();
        let mut scheduler = CloudSyncScheduler::default();
        assert_eq!(scheduler.tick(now, true, false, true, 30, 1), None);
        assert_eq!(scheduler.tick(now, true, false, true, 30, 2), None);
        assert_eq!(
            scheduler.tick(now + Duration::from_secs(29), true, false, true, 30, 2),
            None
        );
        assert_eq!(
            scheduler.tick(now + Duration::from_secs(30), true, false, true, 30, 2),
            Some(CloudSyncTick::LocalChange)
        );
    }

    #[test]
    fn disabling_resets_startup_and_dirty_progress() {
        let now = Instant::now();
        let mut scheduler = CloudSyncScheduler::default();
        let _ = scheduler.tick(now, true, true, true, 5, 0);
        let _ = scheduler.tick(now, true, true, true, 5, 1);
        assert_eq!(scheduler.tick(now, false, true, true, 5, 1), None);
        assert_eq!(
            scheduler.tick(now, true, true, true, 5, 1),
            Some(CloudSyncTick::StartupCheck)
        );
    }
}
