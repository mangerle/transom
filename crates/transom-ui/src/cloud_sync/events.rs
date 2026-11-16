//! 云同步结果事件：更新设置基线、应用目录、生成历史并执行自动后续动作。

use super::actions::provider_key;
use super::common::*;
use super::entity::SyncHistoryRecord;
use super::history::SyncHistoryStatus;
use crate::app::{AppState, TabContent, TabStatus, app_settings, update_app_settings};
use std::time::Instant;
use transom_core::{
    AppError, CloudPullMode, CloudPushMode, ErrorCode, GitSyncResult, SyncAppEvent,
    SyncCheckReport, SyncDocumentSummary, SyncEventMeta, SyncOperation, SyncPullReport,
    SyncPushReport, SyncRelation, SyncTestReport, SyncTrigger,
};

/// 处理控制器回传的全部同步事件。
pub(crate) fn handle_sync_app_event(
    state: &mut AppState,
    event: SyncAppEvent,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    match event {
        SyncAppEvent::CloudSyncChecked { meta, result } => {
            handle_cloud_sync_checked(state, meta, result, cx);
        }
        SyncAppEvent::CloudSyncTested { meta, result } => {
            handle_cloud_sync_tested(state, meta, result, cx);
        }
        SyncAppEvent::CloudSyncPushed {
            meta,
            mode: _,
            result,
        } => handle_cloud_sync_pushed(state, meta, result, cx),
        SyncAppEvent::CloudSyncPulled { meta, mode, result } => {
            handle_cloud_sync_pulled(state, meta, mode, result, window, cx);
        }
        SyncAppEvent::GitSynced { meta, result } => {
            handle_git_synced(state, meta, result, cx);
        }
    }
}

fn handle_cloud_sync_checked(
    state: &mut AppState,
    meta: SyncEventMeta,
    result: Result<SyncCheckReport, AppError>,
    cx: &mut Context<AppShell>,
) {
    state.features.cloud_sync.update(cx, |view, _| {
        view.state.scheduler.mark_check_finished(Instant::now());
    });
    let provider = provider_key(&app_settings(cx).sync);
    match result {
        Ok(report) => {
            update_app_settings(cx, |settings| {
                if let Some(baseline) = settings.sync.active_baseline_mut() {
                    baseline.last_checked_at = Some(report.at);
                }
            });
            persist_sync_settings(state, cx);
            let status = if matches!(
                report.relation,
                SyncRelation::Diverged | SyncRelation::RemoteMissing
            ) {
                SyncHistoryStatus::Conflict
            } else {
                SyncHistoryStatus::Success
            };
            let message = check_message(&report);
            state.features.cloud_sync.update(cx, |view, cx| {
                view.apply_check_result(
                    report.clone(),
                    SyncHistoryRecord {
                        operation: SyncOperation::Check,
                        trigger: meta.trigger,
                        status,
                        provider: &report.provider,
                        duration_ms: meta.duration_ms,
                        sessions: report.local_summary.sessions,
                        folders: report.local_summary.folders,
                        bytes: 0,
                        message: message.clone(),
                    },
                    cx,
                );
            });
            state.ui.status = Some(message);
            follow_up_after_check(state, &report, meta.trigger, cx);
        }
        Err(error) => {
            crate::cloud_sync::CloudSyncActions::new(state).finish_sync_failure(
                SyncOperation::Check,
                meta.trigger,
                &provider,
                localized_error(&error),
                cx,
            );
        }
    }
    cx.notify();
}

fn handle_cloud_sync_tested(
    state: &mut AppState,
    meta: SyncEventMeta,
    result: Result<SyncTestReport, AppError>,
    cx: &mut Context<AppShell>,
) {
    let provider = provider_key(&app_settings(cx).sync);
    match result {
        Ok(report) => {
            let message = if report.remote_present {
                rust_i18n::t!("sync.test_remote_present", provider = report.provider).to_string()
            } else {
                rust_i18n::t!("sync.test_remote_missing", provider = report.provider).to_string()
            };
            state.features.cloud_sync.update(cx, |view, cx| {
                view.finish_success(
                    SyncHistoryRecord {
                        operation: SyncOperation::TestConnection,
                        trigger: meta.trigger,
                        status: SyncHistoryStatus::Success,
                        provider: &report.provider,
                        duration_ms: meta.duration_ms,
                        sessions: 0,
                        folders: 0,
                        bytes: 0,
                        message: message.clone(),
                    },
                    cx,
                );
            });
            state.ui.status = Some(message);
        }
        Err(error) => crate::cloud_sync::CloudSyncActions::new(state).finish_sync_failure(
            SyncOperation::TestConnection,
            meta.trigger,
            &provider,
            localized_error(&error),
            cx,
        ),
    }
    cx.notify();
}

fn handle_cloud_sync_pushed(
    state: &mut AppState,
    meta: SyncEventMeta,
    result: Result<SyncPushReport, AppError>,
    cx: &mut Context<AppShell>,
) {
    let provider = provider_key(&app_settings(cx).sync);
    match result {
        Ok(report) => {
            state.session.catalog = report.catalog;
            state.session.catalog.normalize();
            state.persist_catalog();
            let catalog_revision = state.session.catalog_revision;
            state.refresh_tab_sessions();
            let sessions = report.sessions;
            let folders = report.folders;
            let bytes = report.bytes;
            update_app_settings(cx, |settings| {
                if let Some(baseline) = settings.sync.active_baseline_mut() {
                    baseline.last_push_at = Some(report.at);
                    baseline.last_checked_at = Some(report.at);
                    baseline.last_sync_catalog_hash = Some(report.catalog_hash);
                    baseline.last_remote_hash = Some(report.remote_hash);
                }
            });
            persist_sync_settings(state, cx);
            let message = rust_i18n::t!(
                "sync.pushed",
                sessions = sessions,
                kb = (bytes / 1024).max(1)
            )
            .to_string();
            state.features.cloud_sync.update(cx, |view, cx| {
                view.state.scheduler.mark_catalog_synced(catalog_revision);
                view.state.conflict = None;
                view.finish_success(
                    SyncHistoryRecord {
                        operation: SyncOperation::Push,
                        trigger: meta.trigger,
                        status: SyncHistoryStatus::Success,
                        provider: &provider,
                        duration_ms: meta.duration_ms,
                        sessions,
                        folders,
                        bytes,
                        message: message.clone(),
                    },
                    cx,
                );
            });
            state.ui.status = Some(message);
        }
        Err(error) => {
            let status = if error.code == ErrorCode::CloudSyncConflict {
                SyncHistoryStatus::Conflict
            } else {
                SyncHistoryStatus::Failed
            };
            let message = localized_error(&error);
            state.set_error(message.clone());
            state.features.cloud_sync.update(cx, |view, cx| {
                view.finish_success(
                    SyncHistoryRecord {
                        operation: SyncOperation::Push,
                        trigger: meta.trigger,
                        status,
                        provider: &provider,
                        duration_ms: meta.duration_ms,
                        sessions: 0,
                        folders: 0,
                        bytes: 0,
                        message: message.clone(),
                    },
                    cx,
                );
            });
            state.ui.status = Some(message);
        }
    }
    cx.notify();
}

fn handle_cloud_sync_pulled(
    state: &mut AppState,
    meta: SyncEventMeta,
    mode: CloudPullMode,
    result: Result<SyncPullReport, AppError>,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    let provider = provider_key(&app_settings(cx).sync);
    match result {
        Ok(report) => {
            let summary = SyncDocumentSummary::from_document(&report.document);
            let message = match mode {
                CloudPullMode::Merge => rust_i18n::t!("sync.pull_downloaded").to_string(),
                CloudPullMode::ForceRemote => {
                    rust_i18n::t!("sync.pull_force_downloaded").to_string()
                }
            };
            state.features.cloud_sync.update(cx, |view, cx| {
                view.finish_success(
                    SyncHistoryRecord {
                        operation: SyncOperation::Pull,
                        trigger: meta.trigger,
                        status: SyncHistoryStatus::Success,
                        provider: &provider,
                        duration_ms: meta.duration_ms,
                        sessions: summary.sessions,
                        folders: summary.folders,
                        bytes: report.bytes,
                        message: message.clone(),
                    },
                    cx,
                );
                view.state.conflict = None;
                view.begin_operation(
                    SyncOperation::Pull,
                    meta.trigger,
                    rust_i18n::t!("sync.pull_applying").to_string(),
                    cx,
                );
            });
            state.ui.status = Some(message);
            crate::cloud_sync::finish_cloud_sync_pull(state, report, mode, window, cx);
        }
        Err(error) => {
            crate::cloud_sync::CloudSyncActions::new(state).finish_sync_failure(
                SyncOperation::Pull,
                meta.trigger,
                &provider,
                localized_error(&error),
                cx,
            );
        }
    }
    cx.notify();
}

fn handle_git_synced(
    state: &mut AppState,
    meta: SyncEventMeta,
    result: Result<GitSyncResult, AppError>,
    cx: &mut Context<AppShell>,
) {
    match result {
        Ok(result) => {
            let report = result.report;
            let at = result.at;
            let pushed = result.pushed;
            state.session.catalog = result.catalog;
            state.session.catalog.normalize();
            state.persist_catalog();
            state.refresh_tab_sessions();
            update_app_settings(cx, |settings| {
                if let Some(baseline) = settings.sync.active_baseline_mut() {
                    if pushed {
                        baseline.last_push_at = Some(at);
                    } else {
                        baseline.last_pull_at = Some(at);
                    }
                    baseline.last_checked_at = Some(at);
                }
            });
            persist_sync_settings(state, cx);
            let message = rust_i18n::t!(
                "sync.git_done",
                added = report.sessions_added,
                updated = report.sessions_updated,
                conflicts = report.conflicts
            )
            .to_string();
            state.features.cloud_sync.update(cx, |view, cx| {
                view.finish_success(
                    SyncHistoryRecord {
                        operation: SyncOperation::Git,
                        trigger: meta.trigger,
                        status: SyncHistoryStatus::Success,
                        provider: "git",
                        duration_ms: meta.duration_ms,
                        sessions: report.sessions_added + report.sessions_updated,
                        folders: report.folders_added + report.folders_updated,
                        bytes: result.bytes,
                        message: message.clone(),
                    },
                    cx,
                );
            });
            state.ui.status = Some(message);
        }
        Err(error) => {
            crate::cloud_sync::CloudSyncActions::new(state).finish_sync_failure(
                SyncOperation::Git,
                meta.trigger,
                "git",
                localized_error(&error),
                cx,
            );
        }
    }
    cx.notify();
}

/// 自动检查完成后，仅对可安全推导的方向执行后续动作。
fn follow_up_after_check(
    state: &mut AppState,
    report: &SyncCheckReport,
    trigger: SyncTrigger,
    cx: &mut Context<AppShell>,
) {
    if trigger == SyncTrigger::Manual || trigger == SyncTrigger::ConflictResolution {
        return;
    }
    let settings = app_settings(cx).sync.clone();
    match report.relation {
        SyncRelation::LocalAhead if settings.auto_push_on_change => {
            crate::cloud_sync::CloudSyncActions::new(state).dispatch_cloud_sync_push(
                settings,
                trigger,
                CloudPushMode::Merge,
                cx,
            );
        }
        SyncRelation::RemoteAhead if settings.auto_pull_remote_changes => {
            if let Some(reason) = auto_pull_block_reason(state, cx) {
                state.features.cloud_sync.update(cx, |view, cx| {
                    view.state.message = Some(reason.clone());
                    cx.notify();
                });
                state.ui.status = Some(reason);
            } else {
                crate::cloud_sync::CloudSyncActions::new(state).dispatch_cloud_sync_pull(
                    settings,
                    trigger,
                    CloudPullMode::Merge,
                    cx,
                );
            }
        }
        SyncRelation::UpToDate
        | SyncRelation::Diverged
        | SyncRelation::RemoteMissing
        | SyncRelation::LocalAhead
        | SyncRelation::RemoteAhead => {}
    }
}

/// 自动下拉前确认没有活动终端与未关闭的设置弹窗。
fn auto_pull_block_reason(state: &AppState, cx: &App) -> Option<String> {
    if state.ui.modals.settings.read(cx).is_open() {
        return Some(rust_i18n::t!("sync.auto_pull_settings_open").to_string());
    }
    let has_active_terminal = state.workspace.tabs.iter().any(|tab| {
        matches!(tab.content, TabContent::Session(_))
            && !matches!(tab.status, TabStatus::Failed(_) | TabStatus::Disconnected)
    });
    has_active_terminal.then(|| rust_i18n::t!("sync.auto_pull_sessions_open").to_string())
}

fn persist_sync_settings(state: &mut AppState, cx: &mut Context<AppShell>) {
    let snapshot = app_settings(cx).clone();
    state.persist_settings_reported(&snapshot);
}

fn check_message(report: &SyncCheckReport) -> String {
    let remote_sessions = report
        .remote_summary
        .as_ref()
        .map_or(0, |summary| summary.sessions);
    match report.relation {
        SyncRelation::UpToDate => rust_i18n::t!("sync.check_up_to_date").to_string(),
        SyncRelation::LocalAhead => rust_i18n::t!(
            "sync.check_local_ahead",
            sessions = report.local_summary.sessions
        )
        .to_string(),
        SyncRelation::RemoteAhead => {
            rust_i18n::t!("sync.check_remote_ahead", sessions = remote_sessions).to_string()
        }
        SyncRelation::Diverged => rust_i18n::t!(
            "sync.check_diverged",
            local = report.local_summary.sessions,
            remote = remote_sessions
        )
        .to_string(),
        SyncRelation::RemoteMissing => rust_i18n::t!("sync.check_remote_missing").to_string(),
    }
}
