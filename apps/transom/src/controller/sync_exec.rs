//! 云同步命令的分发：同一时刻只允许一个在途任务，完成后经事件通道回报。
//!
//! 为什么限制单飞：同步对象是全量备份，两个任务并发上传会互相覆盖远端
//! 快照，下载与上传交叉进行也会让合并结果互相覆盖；这里快速拒绝并回报，
//! 由用户稍后重试。事件携带触发来源、开始时间和耗时，UI 可直接生成历史。

use std::sync::Arc;
use std::time::Instant;
use tokio::task::JoinHandle;
use transom_core::{
    AppError, AppEvent, CloudPullMode, CloudPushMode, ErrorCode, SessionCatalog, SyncAppEvent,
    SyncEventMeta, SyncOperation, SyncSettings, SyncTrigger,
};
use transom_sync::CloudSyncService;

/// 一次云同步作业。
pub(super) enum CloudSyncJob {
    Check {
        settings: SyncSettings,
        catalog: SessionCatalog,
        trigger: SyncTrigger,
    },
    Test {
        settings: SyncSettings,
        trigger: SyncTrigger,
    },
    Push {
        settings: SyncSettings,
        catalog: SessionCatalog,
        trigger: SyncTrigger,
        mode: CloudPushMode,
    },
    Pull {
        settings: SyncSettings,
        trigger: SyncTrigger,
        mode: CloudPullMode,
    },
    /// 私有 Git 仓库同步：拉取、三路合并并推送加密信封。
    Git {
        settings: SyncSettings,
        catalog: SessionCatalog,
        trigger: SyncTrigger,
    },
}

impl CloudSyncJob {
    fn operation(&self) -> SyncOperation {
        match self {
            Self::Check { .. } => SyncOperation::Check,
            Self::Test { .. } => SyncOperation::TestConnection,
            Self::Push { .. } => SyncOperation::Push,
            Self::Pull { .. } => SyncOperation::Pull,
            Self::Git { .. } => SyncOperation::Git,
        }
    }

    fn trigger(&self) -> SyncTrigger {
        match self {
            Self::Check { trigger, .. }
            | Self::Test { trigger, .. }
            | Self::Push { trigger, .. }
            | Self::Pull { trigger, .. }
            | Self::Git { trigger, .. } => *trigger,
        }
    }
}

/// 派发云同步作业；在途任务完成后自动把结果事件送回界面。
///
/// 任务句柄登记在 `in_flight`，停机时由命令循环统一中止，杜绝孤儿任务。
pub(super) async fn dispatch_cloud_sync(
    service: &Arc<CloudSyncService>,
    event_tx: &flume::Sender<AppEvent>,
    in_flight: &mut Option<JoinHandle<()>>,
    job: CloudSyncJob,
) {
    let operation = job.operation();
    let trigger = job.trigger();
    if in_flight.as_ref().is_some_and(|task| !task.is_finished()) {
        let error = AppError::with_detail(
            ErrorCode::CloudSyncFailed,
            "已有一次云同步在进行中，请等它完成后重试",
        );
        let meta = SyncEventMeta {
            trigger,
            started_at_ms: chrono::Utc::now().timestamp_millis(),
            duration_ms: 0,
        };
        if event_tx
            .try_send(sync_error_event(operation, meta, error))
            .is_err()
        {
            log::warn!("云同步忙碌提示发送失败：事件通道已满");
        }
        return;
    }

    let service = service.clone();
    let event_tx = event_tx.clone();
    // 云同步跨进程退出没有可恢复的在途事务：停机路径直接中止本任务。
    *in_flight = Some(tokio::spawn(async move {
        let started_at_ms = chrono::Utc::now().timestamp_millis();
        let started = Instant::now();
        let event = match job {
            CloudSyncJob::Check {
                settings,
                catalog,
                trigger,
            } => {
                let result = service.check(&catalog, &settings).await;
                SyncAppEvent::CloudSyncChecked {
                    meta: meta(trigger, started_at_ms, started),
                    result,
                }
            }
            CloudSyncJob::Test { settings, trigger } => {
                let result = service.test_connection(&settings).await;
                SyncAppEvent::CloudSyncTested {
                    meta: meta(trigger, started_at_ms, started),
                    result,
                }
            }
            CloudSyncJob::Push {
                settings,
                catalog,
                trigger,
                mode,
            } => {
                let result = service.push(&catalog, &settings, mode).await;
                SyncAppEvent::CloudSyncPushed {
                    meta: meta(trigger, started_at_ms, started),
                    mode,
                    result,
                }
            }
            CloudSyncJob::Pull {
                settings,
                trigger,
                mode,
            } => {
                let result = service.pull(&settings).await;
                SyncAppEvent::CloudSyncPulled {
                    meta: meta(trigger, started_at_ms, started),
                    mode,
                    result,
                }
            }
            CloudSyncJob::Git {
                settings,
                catalog,
                trigger,
            } => {
                // Git 子进程与 Argon2 派生都是阻塞操作，整段放进阻塞线程池。
                let service = service.clone();
                let cache_dir = transom_platform::paths::config_dir()
                    .join("sync")
                    .join("git");
                let result = tokio::task::spawn_blocking(move || {
                    service.run_git_sync(&settings, &catalog, &cache_dir)
                })
                .await
                .map_err(|error| {
                    AppError::with_detail(
                        ErrorCode::CloudSyncFailed,
                        format!("Git 同步任务异常终止: {error}"),
                    )
                })
                .and_then(|result| result);
                SyncAppEvent::GitSynced {
                    meta: meta(trigger, started_at_ms, started),
                    result,
                }
            }
        };
        if event_tx.send_async(event.into()).await.is_err() {
            log::warn!("云同步结果发送失败：事件通道已关闭");
        }
    }));
}

fn meta(trigger: SyncTrigger, started_at_ms: i64, started: Instant) -> SyncEventMeta {
    SyncEventMeta {
        trigger,
        started_at_ms,
        duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
    }
}

fn sync_error_event(operation: SyncOperation, meta: SyncEventMeta, error: AppError) -> AppEvent {
    match operation {
        SyncOperation::Check => SyncAppEvent::CloudSyncChecked {
            meta,
            result: Err(error),
        }
        .into(),
        SyncOperation::TestConnection => SyncAppEvent::CloudSyncTested {
            meta,
            result: Err(error),
        }
        .into(),
        SyncOperation::Push => SyncAppEvent::CloudSyncPushed {
            meta,
            mode: CloudPushMode::Merge,
            result: Err(error),
        }
        .into(),
        SyncOperation::Pull => SyncAppEvent::CloudSyncPulled {
            meta,
            mode: CloudPullMode::Merge,
            result: Err(error),
        }
        .into(),
        SyncOperation::Git => SyncAppEvent::GitSynced {
            meta,
            result: Err(error),
        }
        .into(),
    }
}
