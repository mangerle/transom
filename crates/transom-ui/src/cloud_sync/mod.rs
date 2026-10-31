//! 云端同步的界面编排：设置表单、凭据落钥匙串、上传下载与合并回填。
//!
//! 分层说明（Why）：
//! - 端点拓扑（地址、账号、目录）进设置文件，WebDAV 密码、S3 Secret Key
//!   与同步口令只进系统钥匙串——设置文件可能被导出共享，秘密绝不同行；
//! - 上传下载是分钟级网络操作，走 `AppCommand` 交给控制器的 tokio 运行时，
//!   结果经 `AppEvent` 回流；合并动作回到界面线程执行，因为目录快照是
//!   界面侧的独占状态；
//! - 合并策略：会话按 updated_at 较新者胜；没有时间戳的文件夹/凭据在冲突时
//!   按操作方向裁决（拉取取远端）。下载合并前仍先导出本机安全副本，误合并
//!   可从文件恢复。
//!
//! 模块按职责拆分：scheduler 只管节拍，form 管输入与凭据，actions 管触发与
//! 完成回调，pull 管拉取后的合并与恢复；外部仍从本模块重导出公共类型。

mod actions;
mod common;
mod form;
mod pull;
mod scheduler;

pub(crate) use form::{DEFAULT_SYNC_DIRECTORY, SyncFormInputs};
pub(crate) use scheduler::GitSyncScheduler;

#[cfg(test)]
mod tests {
    use super::GitSyncScheduler;

    /// 间隔到达才触发，触发后重新计时。
    #[test]
    fn scheduler_fires_at_interval_and_resets() {
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
}
