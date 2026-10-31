//! Git 定时同步节拍器。

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
