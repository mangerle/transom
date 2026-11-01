//! Git/WebDAV/S3 同步触发与完成回调。

use super::common::*;
use crate::app::app_settings;
use crate::app::update_app_settings;

impl AppShell {
    pub(crate) fn start_git_sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let draft = self.state.modals.settings.read(cx).state().clone();
        if let Err(error) = crate::settings::sync_form::persist_sync_credentials(
            &draft,
            self.state.services.secret_store.as_ref(),
            window,
            cx,
        ) {
            self.state.set_error(localized_error(&error));
            cx.notify();
            return;
        }
        let settings = crate::settings::sync_form::collect_sync_settings(&draft, cx);
        if settings.git.is_none() {
            self.state
                .set_error(rust_i18n::t!("sync.git_no_config").to_string());
            cx.notify();
            return;
        }
        self.send_git_sync(settings, cx);
    }

    /// 定时同步入口：秘密已经在钥匙串里，不再触碰设置表单。
    pub(crate) fn start_git_sync_scheduled(&mut self, cx: &mut Context<Self>) {
        let settings = app_settings(cx).sync.clone();
        if settings.git.is_none() {
            return;
        }
        self.send_git_sync(settings, cx);
    }

    fn send_git_sync(&mut self, settings: SyncSettings, cx: &mut Context<Self>) {
        self.state.status = Some(rust_i18n::t!("sync.git_running").to_string());
        let catalog = self.state.catalog.clone();
        if !self
            .state
            .send_command(SyncCommand::GitSync { settings, catalog })
        {
            self.state
                .set_error(rust_i18n::t!("sync.git_no_config").to_string());
        }
        cx.notify();
    }

    /// Git 同步完成：整体替换本地目录并刷新已打开标签。
    pub(crate) fn handle_git_synced(
        &mut self,
        result: Result<GitSyncResult, AppError>,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(result) => {
                let report = result.report;
                self.state.catalog = result.catalog;
                self.state.catalog.normalize();
                self.state.persist_catalog();
                self.state.refresh_tab_sessions();
                update_app_settings(cx, |s| s.sync.last_pull_at = Some(result.at));
                self.state.status = Some(
                    rust_i18n::t!(
                        "sync.git_done",
                        added = report.sessions_added,
                        updated = report.sessions_updated,
                        conflicts = report.conflicts
                    )
                    .to_string(),
                );
            }
            Err(error) => {
                self.state.set_error(localized_error(&error));
            }
        }
        cx.notify();
    }

    /// 立即上传：先落凭据，再以输入框现状发起云端推送。
    pub(crate) fn start_cloud_sync_push(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let draft = self.state.modals.settings.read(cx).state().clone();
        if let Err(error) = crate::settings::sync_form::persist_sync_credentials(
            &draft,
            self.state.services.secret_store.as_ref(),
            window,
            cx,
        ) {
            self.state.set_error(localized_error(&error));
            cx.notify();
            return;
        }
        let settings = crate::settings::sync_form::collect_sync_settings(&draft, cx);
        if settings.endpoint.is_none() {
            self.state
                .set_error(rust_i18n::t!("sync.error_no_endpoint").to_string());
            cx.notify();
            return;
        }
        let catalog = self.state.catalog.clone();
        if !self
            .state
            .send_command(SyncCommand::CloudSyncPush { settings, catalog })
        {
            return;
        }
        self.state.status = Some(rust_i18n::t!("sync.push_started").to_string());
        cx.notify();
    }

    /// 立即下载：流程同上传，远端文档由事件回流的合并动作接手。
    pub(crate) fn start_cloud_sync_pull(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let draft = self.state.modals.settings.read(cx).state().clone();
        if let Err(error) = crate::settings::sync_form::persist_sync_credentials(
            &draft,
            self.state.services.secret_store.as_ref(),
            window,
            cx,
        ) {
            self.state.set_error(localized_error(&error));
            cx.notify();
            return;
        }
        let settings = crate::settings::sync_form::collect_sync_settings(&draft, cx);
        if settings.endpoint.is_none() {
            self.state
                .set_error(rust_i18n::t!("sync.error_no_endpoint").to_string());
            cx.notify();
            return;
        }
        if !self
            .state
            .send_command(SyncCommand::CloudSyncPull { settings })
        {
            return;
        }
        self.state.status = Some(rust_i18n::t!("sync.pull_started").to_string());
        cx.notify();
    }

    /// 上传完成：回填时间戳并落盘设置。
    pub(crate) fn handle_cloud_sync_pushed(
        &mut self,
        result: Result<SyncPushReport, AppError>,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(report) => {
                update_app_settings(cx, |s| s.sync.last_push_at = Some(report.at));
                let settings_snapshot = app_settings(cx).clone();
                if let Err(error) = self.state.persist_settings(&settings_snapshot) {
                    self.state.set_error(localized_error(&error));
                }
                self.state.status = Some(
                    rust_i18n::t!(
                        "sync.pushed",
                        sessions = report.sessions,
                        kb = (report.bytes / 1024).max(1)
                    )
                    .to_string(),
                );
            }
            Err(error) => {
                self.state.set_error(localized_error(&error));
            }
        }
        cx.notify();
    }
}
