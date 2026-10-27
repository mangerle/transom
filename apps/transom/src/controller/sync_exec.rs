//! 云同步命令的分发：同一时刻只允许一个在途任务，完成后经事件通道回报。
//!
//! 为什么限制单飞：同步对象是全量备份，两个任务并发上传会互相覆盖远端
//! 快照，下载与上传交叉进行也会让"后写者胜"的冲突策略失去语义；这里
//! 快速拒绝并回报，由用户稍后重试。

use std::sync::Arc;
use tokio::task::JoinHandle;
use transom_core::{AppError, AppEvent, ErrorCode, SessionCatalog, SyncSettings};
use transom_sync::CloudSyncService;

/// 一次云同步作业：上传目录快照或下载合并远端备份。
pub(super) enum CloudSyncJob {
    Push {
        settings: SyncSettings,
        catalog: SessionCatalog,
    },
    Pull {
        settings: SyncSettings,
    },
    /// 私有 Git 仓库同步：拉取、三路合并并推送加密信封。
    Git {
        settings: SyncSettings,
        catalog: SessionCatalog,
    },
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
    if in_flight.as_ref().is_some_and(|task| !task.is_finished()) {
        let error = AppError::with_detail(
            ErrorCode::CloudSyncFailed,
            "已有一次云同步在进行中，请等它完成后重试",
        );
        let event = match job {
            CloudSyncJob::Push { .. } => AppEvent::CloudSyncPushed { result: Err(error) },
            CloudSyncJob::Pull { .. } => AppEvent::CloudSyncPulled { result: Err(error) },
            CloudSyncJob::Git { .. } => AppEvent::GitSynced { result: Err(error) },
        };
        if event_tx.try_send(event).is_err() {
            log::warn!("云同步忙碌提示发送失败：事件通道已满");
        }
        return;
    }

    let service = service.clone();
    let event_tx = event_tx.clone();
    // 云同步跨进程退出没有可恢复的在途事务：停机路径直接中止本任务。
    *in_flight = Some(tokio::spawn(async move {
        match job {
            CloudSyncJob::Push { settings, catalog } => {
                let result = service.push(&catalog, &settings).await;
                let _ = event_tx
                    .send_async(AppEvent::CloudSyncPushed { result })
                    .await;
            }
            CloudSyncJob::Pull { settings } => {
                let result = service.pull(&settings).await;
                let _ = event_tx
                    .send_async(AppEvent::CloudSyncPulled { result })
                    .await;
            }
            CloudSyncJob::Git { settings, catalog } => {
                // Git 子进程与 Argon2 派生都是阻塞操作，整段放进阻塞线程池。
                let service = service.clone();
                let cache_dir = transom_core::paths::config_dir().join("sync").join("git");
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
                let _ = event_tx.send_async(AppEvent::GitSynced { result }).await;
            }
        }
    }));
}
