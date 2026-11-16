//! Git/WebDAV/S3 同步命令的采集、触发与通用历史记录。
//!
//! 设计原理（Why）：同步动作同时涉及设置表单、命令通道、目录快照与云同步实体。
//! [`CloudSyncActions`] 把这些依赖收敛成一个窄操作上下文，动作函数不再接收
//! `AppShell`，从而无法顺手读写无关应用状态，也便于独立验证命令入队和失败回执。

use super::CloudSyncView;
use super::common::*;
use crate::app::{
    AppShell, AppState, PendingNotices, Persistence, app_settings, command_send_error_text,
    update_app_settings,
};
use crate::settings::SettingsView;
use gpui_kit::{Context, Entity, Window};
use std::sync::Arc;
use transom_core::{
    AppCommand, CloudPullMode, CloudPushMode, SecretStore, SessionCatalog, SyncCommand,
    SyncOperation, SyncSettings, SyncTrigger,
};

/// 云同步动作所需的窄上下文。
pub(crate) struct CloudSyncActions<'a> {
    cloud_sync: &'a Entity<CloudSyncView>,
    settings_view: &'a Entity<SettingsView>,
    secret_store: &'a Arc<dyn SecretStore>,
    catalog: &'a SessionCatalog,
    command_tx: &'a flume::Sender<AppCommand>,
    pending_notices: &'a mut PendingNotices,
    persistence: &'a Persistence,
    status: &'a mut Option<String>,
}

impl<'a> CloudSyncActions<'a> {
    /// 从应用状态拆借云同步动作能力。
    pub(crate) fn new(state: &'a mut AppState) -> Self {
        Self {
            cloud_sync: &state.features.cloud_sync,
            settings_view: &state.ui.modals.settings,
            secret_store: &state.runtime.services.secret_store,
            catalog: &state.session.catalog,
            command_tx: &state.runtime.services.command_tx,
            pending_notices: &mut state.ui.feedback.pending_notices,
            persistence: &state.runtime.persistence,
            status: &mut state.ui.status,
        }
    }

    /// 设置页的“立即测试”：先保存当前凭据与拓扑，再执行只读探测。
    pub(crate) fn start_cloud_sync_test(
        &mut self,
        window: &mut Window,
        cx: &mut Context<AppShell>,
    ) {
        let Some(settings) = self.manual_sync_settings(window, cx) else {
            return;
        };
        self.dispatch_cloud_sync_test(settings, SyncTrigger::Manual, cx);
    }

    /// 设置页或侧边栏的“检查状态”。
    pub(crate) fn start_cloud_sync_check(
        &mut self,
        window: &mut Window,
        cx: &mut Context<AppShell>,
    ) {
        let Some(settings) = self.manual_sync_settings(window, cx) else {
            return;
        };
        self.dispatch_cloud_sync_check(settings, SyncTrigger::Manual, cx);
    }

    /// 立即上传：常规模式会把远端合并结果回填本地。
    pub(crate) fn start_cloud_sync_push(
        &mut self,
        window: &mut Window,
        cx: &mut Context<AppShell>,
    ) {
        let Some(settings) = self.manual_sync_settings(window, cx) else {
            return;
        };
        self.dispatch_cloud_sync_push(settings, SyncTrigger::Manual, CloudPushMode::Merge, cx);
    }

    /// 立即下载：常规模式把远端变化合并到本地。
    pub(crate) fn start_cloud_sync_pull(
        &mut self,
        window: &mut Window,
        cx: &mut Context<AppShell>,
    ) {
        let Some(settings) = self.manual_sync_settings(window, cx) else {
            return;
        };
        self.dispatch_cloud_sync_pull(settings, SyncTrigger::Manual, CloudPullMode::Merge, cx);
    }

    /// 冲突处理：以当前本机目录覆盖远端。
    pub(crate) fn start_cloud_sync_force_push(&mut self, cx: &mut Context<AppShell>) {
        let settings = app_settings(cx).sync.clone();
        if !self.ensure_endpoint(&settings, cx) {
            return;
        }
        self.dispatch_cloud_sync_push(
            settings,
            SyncTrigger::ConflictResolution,
            CloudPushMode::ForceLocal,
            cx,
        );
    }

    /// 冲突处理：以远端目录整体替换本机；替换前仍写安全副本。
    pub(crate) fn start_cloud_sync_force_pull(&mut self, cx: &mut Context<AppShell>) {
        let settings = app_settings(cx).sync.clone();
        if !self.ensure_endpoint(&settings, cx) {
            return;
        }
        self.dispatch_cloud_sync_pull(
            settings,
            SyncTrigger::ConflictResolution,
            CloudPullMode::ForceRemote,
            cx,
        );
    }

    /// 启动或定时触发的 Git 同步。
    pub(crate) fn start_git_sync(&mut self, window: &mut Window, cx: &mut Context<AppShell>) {
        let draft = self.settings_view.read(cx).state().clone();
        if let Err(error) = crate::settings::sync_form::persist_sync_credentials(
            &draft,
            self.secret_store.as_ref(),
            window,
            cx,
        ) {
            self.set_error(localized_error(&error), cx);
            return;
        }
        let settings = crate::settings::sync_form::collect_sync_settings(&draft, cx);
        if !settings.git_enabled() {
            self.set_error(rust_i18n::t!("sync.git_no_config").to_string(), cx);
            return;
        }
        update_app_settings(cx, |current| current.sync = settings.clone());
        let snapshot = app_settings(cx).clone();
        self.persist_settings_reported(&snapshot);
        self.send_git_sync(settings, SyncTrigger::Manual, cx);
    }

    /// 定时 Git 同步入口：秘密已经在钥匙串里，不再触碰设置表单。
    pub(crate) fn start_git_sync_scheduled(&mut self, cx: &mut Context<AppShell>) {
        let settings = app_settings(cx).sync.clone();
        if settings.git_enabled() {
            self.send_git_sync(settings, SyncTrigger::GitInterval, cx);
        }
    }

    fn send_git_sync(
        &mut self,
        settings: SyncSettings,
        trigger: SyncTrigger,
        cx: &mut Context<AppShell>,
    ) {
        self.begin_sync_operation(
            SyncOperation::Git,
            trigger,
            rust_i18n::t!("sync.git_running").to_string(),
            cx,
        );
        let catalog = self.catalog.clone();
        if !self.send_command(SyncCommand::GitSync {
            settings,
            catalog,
            trigger,
        }) {
            self.finish_sync_failure(
                SyncOperation::Git,
                trigger,
                "git",
                rust_i18n::t!("sync.enqueue_failed").to_string(),
                cx,
            );
        }
        cx.notify();
    }

    /// 使用设置表单现状准备一次手动动作，并把拓扑写入 Global 与设置文件。
    fn manual_sync_settings(
        &mut self,
        window: &mut Window,
        cx: &mut Context<AppShell>,
    ) -> Option<SyncSettings> {
        let draft = self.settings_view.read(cx).state().clone();
        if let Err(error) = crate::settings::sync_form::persist_sync_credentials(
            &draft,
            self.secret_store.as_ref(),
            window,
            cx,
        ) {
            self.set_error(localized_error(&error), cx);
            return None;
        }
        let settings = crate::settings::sync_form::collect_sync_settings(&draft, cx);
        if !settings.enabled {
            self.set_error(rust_i18n::t!("sync.disabled_action").to_string(), cx);
            return None;
        }
        if !self.ensure_endpoint(&settings, cx) {
            return None;
        }
        update_app_settings(cx, |current| current.sync = settings.clone());
        let snapshot = app_settings(cx).clone();
        self.persist_settings_reported(&snapshot);
        Some(settings)
    }

    fn ensure_endpoint(&mut self, settings: &SyncSettings, cx: &mut Context<AppShell>) -> bool {
        if settings.has_endpoint() {
            true
        } else {
            self.set_error(rust_i18n::t!("sync.error_no_endpoint").to_string(), cx);
            false
        }
    }

    /// 触发一次云端状态检查。
    pub(crate) fn dispatch_cloud_sync_check(
        &mut self,
        settings: SyncSettings,
        trigger: SyncTrigger,
        cx: &mut Context<AppShell>,
    ) {
        self.dispatch_cloud_command(
            SyncOperation::Check,
            trigger,
            rust_i18n::t!("sync.checking").to_string(),
            provider_key(&settings),
            |catalog| SyncCommand::CloudSyncCheck {
                settings,
                catalog,
                trigger,
            },
            cx,
        );
    }

    /// 触发一次只读连接测试。
    pub(crate) fn dispatch_cloud_sync_test(
        &mut self,
        settings: SyncSettings,
        trigger: SyncTrigger,
        cx: &mut Context<AppShell>,
    ) {
        self.dispatch_cloud_command(
            SyncOperation::TestConnection,
            trigger,
            rust_i18n::t!("sync.testing").to_string(),
            provider_key(&settings),
            |_| SyncCommand::CloudSyncTest { settings, trigger },
            cx,
        );
    }

    /// 触发一次云端上传。
    pub(crate) fn dispatch_cloud_sync_push(
        &mut self,
        settings: SyncSettings,
        trigger: SyncTrigger,
        mode: CloudPushMode,
        cx: &mut Context<AppShell>,
    ) {
        self.dispatch_cloud_command(
            SyncOperation::Push,
            trigger,
            rust_i18n::t!("sync.push_started").to_string(),
            provider_key(&settings),
            |catalog| SyncCommand::CloudSyncPush {
                settings,
                catalog,
                trigger,
                mode,
            },
            cx,
        );
    }

    /// 触发一次云端下载。
    pub(crate) fn dispatch_cloud_sync_pull(
        &mut self,
        settings: SyncSettings,
        trigger: SyncTrigger,
        mode: CloudPullMode,
        cx: &mut Context<AppShell>,
    ) {
        self.dispatch_cloud_command(
            SyncOperation::Pull,
            trigger,
            rust_i18n::t!("sync.pull_started").to_string(),
            provider_key(&settings),
            |_| SyncCommand::CloudSyncPull {
                settings,
                trigger,
                mode,
            },
            cx,
        );
    }

    /// 统一登记开始状态并投递命令，命令满载时立即写入失败历史。
    fn dispatch_cloud_command(
        &mut self,
        operation: SyncOperation,
        trigger: SyncTrigger,
        message: String,
        provider: String,
        make_command: impl FnOnce(SessionCatalog) -> SyncCommand,
        cx: &mut Context<AppShell>,
    ) {
        self.begin_sync_operation(operation, trigger, message, cx);
        if !self.send_command(make_command(self.catalog.clone())) {
            self.finish_sync_failure(
                operation,
                trigger,
                &provider,
                rust_i18n::t!("sync.enqueue_failed").to_string(),
                cx,
            );
        }
        cx.notify();
    }

    fn begin_sync_operation(
        &mut self,
        operation: SyncOperation,
        trigger: SyncTrigger,
        message: String,
        cx: &mut Context<AppShell>,
    ) {
        self.cloud_sync.update(cx, |view, cx| {
            view.begin_operation(operation, trigger, message.clone(), cx);
        });
        *self.status = Some(message);
    }

    /// 记录一次失败并保留触发上下文；底层错误由事件处理层传入本地化文本。
    pub(crate) fn finish_sync_failure(
        &mut self,
        operation: SyncOperation,
        trigger: SyncTrigger,
        provider: &str,
        message: String,
        cx: &mut Context<AppShell>,
    ) {
        self.set_error(message.clone(), cx);
        *self.status = self.cloud_sync.update(cx, |view, cx| {
            view.finish_failure(operation, trigger, provider, message, cx)
        });
    }

    fn send_command(&mut self, command: impl Into<AppCommand>) -> bool {
        match self.command_tx.try_send(command.into()) {
            Ok(()) => true,
            Err(error) => {
                self.pending_notices.push(command_send_error_text(error));
                false
            }
        }
    }

    fn set_error(&mut self, message: String, cx: &mut Context<AppShell>) {
        self.pending_notices.push(message);
        cx.notify();
    }

    fn persist_settings_reported(&mut self, settings: &transom_core::AppSettings) {
        if let Err(error) = self.persistence.save_settings(settings) {
            log::error!("保存云同步设置失败，界面仍是新值但下次启动会回退，原因: {error}");
            self.pending_notices
                .push(crate::palette::localized_error(&error));
        }
    }
}

/// 当前设置对应的同步提供方标识；无端点时返回稳定的兜底键。
pub(crate) fn provider_key(settings: &SyncSettings) -> String {
    settings
        .active_endpoint()
        .map(|endpoint| endpoint.provider_key().to_string())
        .or_else(|| {
            (settings.active_channel == transom_core::SyncChannel::Git).then(|| "git".to_string())
        })
        .unwrap_or_else(|| "none".to_string())
}
