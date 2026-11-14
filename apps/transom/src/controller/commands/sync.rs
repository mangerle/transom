//! 云同步命令处理。

use super::super::state::{ControllerCtx, ControllerState};
use super::super::sync_exec::{CloudSyncJob, dispatch_cloud_sync};
use transom_core::SyncCommand;

pub(super) async fn handle_sync_command(
    state: &mut ControllerState,
    ctx: &ControllerCtx,
    command: SyncCommand,
) {
    match command {
        SyncCommand::CloudSyncCheck {
            settings,
            catalog,
            trigger,
        } => {
            dispatch_cloud_sync(
                &ctx.cloud_sync,
                &ctx.event_tx,
                &mut state.sync_task,
                CloudSyncJob::Check {
                    settings,
                    catalog,
                    trigger,
                },
            )
            .await;
        }
        SyncCommand::CloudSyncTest { settings, trigger } => {
            dispatch_cloud_sync(
                &ctx.cloud_sync,
                &ctx.event_tx,
                &mut state.sync_task,
                CloudSyncJob::Test { settings, trigger },
            )
            .await;
        }
        SyncCommand::CloudSyncPush {
            settings,
            catalog,
            trigger,
            mode,
        } => {
            dispatch_cloud_sync(
                &ctx.cloud_sync,
                &ctx.event_tx,
                &mut state.sync_task,
                CloudSyncJob::Push {
                    settings,
                    catalog,
                    trigger,
                    mode,
                },
            )
            .await;
        }
        SyncCommand::CloudSyncPull {
            settings,
            trigger,
            mode,
        } => {
            dispatch_cloud_sync(
                &ctx.cloud_sync,
                &ctx.event_tx,
                &mut state.sync_task,
                CloudSyncJob::Pull {
                    settings,
                    trigger,
                    mode,
                },
            )
            .await;
        }
        SyncCommand::GitSync {
            settings,
            catalog,
            trigger,
        } => {
            dispatch_cloud_sync(
                &ctx.cloud_sync,
                &ctx.event_tx,
                &mut state.sync_task,
                CloudSyncJob::Git {
                    settings,
                    catalog,
                    trigger,
                },
            )
            .await;
        }
    }
}

