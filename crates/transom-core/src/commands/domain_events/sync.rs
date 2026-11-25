//! 云端备份与 Git 同步事件。

use crate::error::AppError;
use crate::protocol::sync::{
    CloudPullMode, CloudPushMode, GitSyncResult, SyncCheckReport, SyncEventMeta, SyncPullReport,
    SyncPushReport, SyncTestReport,
};

/// 云端备份与 Git 同步事件。
///
/// 每个事件都携带触发来源、开始时间与耗时，UI 无需从全局状态反推历史上下文。
pub enum SyncAppEvent {
    CloudSyncChecked {
        meta: SyncEventMeta,
        result: Result<SyncCheckReport, AppError>,
    },
    CloudSyncTested {
        meta: SyncEventMeta,
        result: Result<SyncTestReport, AppError>,
    },
    CloudSyncPushed {
        meta: SyncEventMeta,
        mode: CloudPushMode,
        result: Result<SyncPushReport, AppError>,
    },
    CloudSyncPulled {
        meta: SyncEventMeta,
        mode: CloudPullMode,
        result: Result<SyncPullReport, AppError>,
    },
    GitSynced {
        meta: SyncEventMeta,
        result: Result<GitSyncResult, AppError>,
    },
}
