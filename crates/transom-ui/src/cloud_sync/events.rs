//! 云同步结果事件：更新设置基线、应用目录、生成历史并执行自动后续动作。

use super::actions::{SyncHistoryRecord, provider_key};
use super::common::*;
use super::history::SyncHistoryStatus;
use crate::app::{TabContent, TabStatus, app_settings, update_app_settings};
use std::time::Instant;
use transom_core::{
    AppError, CloudPullMode, CloudPushMode, ErrorCode, GitSyncResult, SyncAppEvent,
    SyncCheckReport, SyncDocumentSummary, SyncEventMeta, SyncOperation, SyncPullReport,
    SyncPushReport, SyncRelation, SyncTestReport, SyncTrigger,
};

impl AppShell {
    /// 处理控制器回传的全部同步事件。
    pub(crate) fn handle_sync_app_event(
        &mut self,
        event: SyncAppEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            SyncAppEvent::CloudSyncChecked { meta, result } => {
                self.handle_cloud_sync_checked(meta, result, cx);
            }
            SyncAppEvent::CloudSyncTested { meta, result } => {
                self.handle_cloud_sync_tested(meta, result, cx);
            }
            SyncAppEvent::CloudSyncPushed {
                meta,
                mode: _,
                result,
            } => self.handle_cloud_sync_pushed(meta, result, cx),
            SyncAppEvent::CloudSyncPulled { meta, mode, result } => {
                self.handle_cloud_sync_pulled(meta, mode, result, window, cx);
            }
            SyncAppEvent::GitSynced { meta, result } => {
                self.handle_git_synced(meta, result, cx);
            }
        }
    }

    fn handle_cloud_sync_checked(
        &mut self,
        meta: SyncEventMeta,
        result: Result<SyncCheckReport, AppError>,
        cx: &mut Context<Self>,
    ) {
        self.state
            .cloud_sync
            .scheduler
            .mark_check_finished(Instant::now());
        let provider = provider_key(&app_settings(cx).sync);
        match result {
            Ok(report) => {
                update_app_settings(cx, |settings| {
                    if let Some(baseline) = settings.sync.active_baseline_mut() {
                        baseline.last_checked_at = Some(report.at);
                    }
                });
                self.persist_sync_settings(cx);
                self.state.cloud_sync.apply_check(report.clone());
                let status = if matches!(
                    report.relation,
                    SyncRelation::Diverged | SyncRelation::RemoteMissing
                ) {
                    SyncHistoryStatus::Conflict
                } else {
                    SyncHistoryStatus::Success
                };
                let message = check_message(&report);
                let entry = self.record_sync_history(SyncHistoryRecord {
                    operation: SyncOperation::Check,
                    trigger: meta.trigger,
                    status,
                    provider: &report.provider,
                    duration_ms: meta.duration_ms,
                    sessions: report.local_summary.sessions,
                    folders: report.local_summary.folders,
                    bytes: 0,
                    message: message.clone(),
                });
                self.state.cloud_sync.finish(entry);
                self.state.status = Some(message);
                self.follow_up_after_check(&report, meta.trigger, cx);
            }
            Err(error) => {
                self.finish_sync_failure(
                    SyncOperation::Check,
                    meta.trigger,
                    &provider,
                    localized_error(&error),
                );
            }
        }
        cx.notify();
    }

    fn handle_cloud_sync_tested(
        &mut self,
        meta: SyncEventMeta,
        result: Result<SyncTestReport, AppError>,
        cx: &mut Context<Self>,
    ) {
        let provider = provider_key(&app_settings(cx).sync);
        match result {
            Ok(report) => {
                let message = if report.remote_present {
                    rust_i18n::t!("sync.test_remote_present", provider = report.provider)
                        .to_string()
                } else {
                    rust_i18n::t!("sync.test_remote_missing", provider = report.provider)
                        .to_string()
                };
                let entry = self.record_sync_history(SyncHistoryRecord {
                    operation: SyncOperation::TestConnection,
                    trigger: meta.trigger,
                    status: SyncHistoryStatus::Success,
                    provider: &report.provider,
                    duration_ms: meta.duration_ms,
                    sessions: 0,
                    folders: 0,
                    bytes: 0,
                    message: message.clone(),
                });
                self.state.cloud_sync.finish(entry);
                self.state.status = Some(message);
            }
            Err(error) => self.finish_sync_failure(
                SyncOperation::TestConnection,
                meta.trigger,
                &provider,
                localized_error(&error),
            ),
        }
        cx.notify();
    }

    fn handle_cloud_sync_pushed(
        &mut self,
        meta: SyncEventMeta,
        result: Result<SyncPushReport, AppError>,
        cx: &mut Context<Self>,
    ) {
        let provider = provider_key(&app_settings(cx).sync);
        match result {
            Ok(report) => {
                self.state.catalog = report.catalog;
                self.state.catalog.normalize();
                self.state.persist_catalog();
                self.state
                    .cloud_sync
                    .scheduler
                    .mark_catalog_synced(self.state.catalog_revision);
                self.state.refresh_tab_sessions();
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
                self.persist_sync_settings(cx);
                self.state.cloud_sync.conflict = None;
                let message = rust_i18n::t!(
                    "sync.pushed",
                    sessions = sessions,
                    kb = (bytes / 1024).max(1)
                )
                .to_string();
                let entry = self.record_sync_history(SyncHistoryRecord {
                    operation: SyncOperation::Push,
                    trigger: meta.trigger,
                    status: SyncHistoryStatus::Success,
                    provider: &provider,
                    duration_ms: meta.duration_ms,
                    sessions,
                    folders,
                    bytes,
                    message: message.clone(),
                });
                self.state.cloud_sync.finish(entry);
                self.state.status = Some(message);
            }
            Err(error) => {
                let status = if error.code == ErrorCode::CloudSyncConflict {
                    SyncHistoryStatus::Conflict
                } else {
                    SyncHistoryStatus::Failed
                };
                let message = localized_error(&error);
                self.state.set_error(message.clone());
                let entry = self.record_sync_history(SyncHistoryRecord {
                    operation: SyncOperation::Push,
                    trigger: meta.trigger,
                    status,
                    provider: &provider,
                    duration_ms: meta.duration_ms,
                    sessions: 0,
                    folders: 0,
                    bytes: 0,
                    message: message.clone(),
                });
                self.state.cloud_sync.finish(entry);
                self.state.status = Some(message);
            }
        }
        cx.notify();
    }

    fn handle_cloud_sync_pulled(
        &mut self,
        meta: SyncEventMeta,
        mode: CloudPullMode,
        result: Result<SyncPullReport, AppError>,
        window: &mut Window,
        cx: &mut Context<Self>,
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
                let entry = self.record_sync_history(SyncHistoryRecord {
                    operation: SyncOperation::Pull,
                    trigger: meta.trigger,
                    status: SyncHistoryStatus::Success,
                    provider: &provider,
                    duration_ms: meta.duration_ms,
                    sessions: summary.sessions,
                    folders: summary.folders,
                    bytes: report.bytes,
                    message: message.clone(),
                });
                self.state.cloud_sync.finish(entry);
                self.state.cloud_sync.conflict = None;
                self.state.cloud_sync.begin(
                    SyncOperation::Pull,
                    meta.trigger,
                    rust_i18n::t!("sync.pull_applying").to_string(),
                );
                self.state.status = Some(message);
                self.finish_cloud_sync_pull(report, mode, window, cx);
            }
            Err(error) => {
                self.finish_sync_failure(
                    SyncOperation::Pull,
                    meta.trigger,
                    &provider,
                    localized_error(&error),
                );
            }
        }
        cx.notify();
    }

    fn handle_git_synced(
        &mut self,
        meta: SyncEventMeta,
        result: Result<GitSyncResult, AppError>,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(result) => {
                let report = result.report;
                let at = result.at;
                let pushed = result.pushed;
                self.state.catalog = result.catalog;
                self.state.catalog.normalize();
                self.state.persist_catalog();
                self.state.refresh_tab_sessions();
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
                self.persist_sync_settings(cx);
                let message = rust_i18n::t!(
                    "sync.git_done",
                    added = report.sessions_added,
                    updated = report.sessions_updated,
                    conflicts = report.conflicts
                )
                .to_string();
                let entry = self.record_sync_history(SyncHistoryRecord {
                    operation: SyncOperation::Git,
                    trigger: meta.trigger,
                    status: SyncHistoryStatus::Success,
                    provider: "git",
                    duration_ms: meta.duration_ms,
                    sessions: report.sessions_added + report.sessions_updated,
                    folders: report.folders_added + report.folders_updated,
                    bytes: result.bytes,
                    message: message.clone(),
                });
                self.state.cloud_sync.finish(entry);
                self.state.status = Some(message);
            }
            Err(error) => {
                self.finish_sync_failure(
                    SyncOperation::Git,
                    meta.trigger,
                    "git",
                    localized_error(&error),
                );
            }
        }
        cx.notify();
    }

    /// 自动检查完成后，仅对可安全推导的方向执行后续动作。
    fn follow_up_after_check(
        &mut self,
        report: &SyncCheckReport,
        trigger: SyncTrigger,
        cx: &mut Context<Self>,
    ) {
        if trigger == SyncTrigger::Manual || trigger == SyncTrigger::ConflictResolution {
            return;
        }
        let settings = app_settings(cx).sync.clone();
        match report.relation {
            SyncRelation::LocalAhead if settings.auto_push_on_change => {
                self.dispatch_cloud_sync_push(settings, trigger, CloudPushMode::Merge, cx);
            }
            SyncRelation::RemoteAhead if settings.auto_pull_remote_changes => {
                if let Some(reason) = self.auto_pull_block_reason(cx) {
                    self.state.cloud_sync.message = Some(reason.clone());
                    self.state.status = Some(reason);
                } else {
                    self.dispatch_cloud_sync_pull(settings, trigger, CloudPullMode::Merge, cx);
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
    fn auto_pull_block_reason(&self, cx: &App) -> Option<String> {
        if self.state.modals.settings.read(cx).is_open() {
            return Some(rust_i18n::t!("sync.auto_pull_settings_open").to_string());
        }
        let has_active_terminal = self.state.workspace.tabs.iter().any(|tab| {
            matches!(tab.content, TabContent::Session(_))
                && !matches!(tab.status, TabStatus::Failed(_) | TabStatus::Disconnected)
        });
        has_active_terminal.then(|| rust_i18n::t!("sync.auto_pull_sessions_open").to_string())
    }

    fn persist_sync_settings(&mut self, cx: &mut Context<Self>) {
        let snapshot = app_settings(cx).clone();
        self.state.persist_settings_reported(&snapshot);
    }
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
