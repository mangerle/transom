//! Git/WebDAV/S3 同步命令的采集、触发与通用历史记录。

use super::common::*;
use super::history::{SyncHistoryEntry, SyncHistoryStatus};
use crate::app::app_settings;
use crate::app::update_app_settings;
use transom_core::{CloudPullMode, CloudPushMode, SyncOperation, SyncTrigger};

/// 设置页的“立即测试”：先保存当前凭据与拓扑，再执行只读探测。
pub(crate) fn start_cloud_sync_test(
    shell: &mut AppShell,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    let Some(settings) = manual_sync_settings(shell, window, cx) else {
        return;
    };
    crate::cloud_sync::dispatch_cloud_sync_test(shell, settings, SyncTrigger::Manual, cx);
}

/// 设置页或侧边栏的“检查状态”。
pub(crate) fn start_cloud_sync_check(
    shell: &mut AppShell,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    let Some(settings) = manual_sync_settings(shell, window, cx) else {
        return;
    };
    crate::cloud_sync::dispatch_cloud_sync_check(shell, settings, SyncTrigger::Manual, cx);
}

/// 立即上传：常规模式会把远端合并结果回填本地。
pub(crate) fn start_cloud_sync_push(
    shell: &mut AppShell,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    let Some(settings) = manual_sync_settings(shell, window, cx) else {
        return;
    };
    crate::cloud_sync::dispatch_cloud_sync_push(
        shell,
        settings,
        SyncTrigger::Manual,
        CloudPushMode::Merge,
        cx,
    );
}

/// 立即下载：常规模式把远端变化合并到本地。
pub(crate) fn start_cloud_sync_pull(
    shell: &mut AppShell,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    let Some(settings) = manual_sync_settings(shell, window, cx) else {
        return;
    };
    crate::cloud_sync::dispatch_cloud_sync_pull(
        shell,
        settings,
        SyncTrigger::Manual,
        CloudPullMode::Merge,
        cx,
    );
}

/// 冲突处理：以当前本机目录覆盖远端。
pub(crate) fn start_cloud_sync_force_push(shell: &mut AppShell, cx: &mut Context<AppShell>) {
    let settings = app_settings(cx).sync.clone();
    if !settings.endpoint_enabled() {
        shell
            .state
            .set_error(rust_i18n::t!("sync.error_no_endpoint").to_string());
        cx.notify();
        return;
    }
    crate::cloud_sync::dispatch_cloud_sync_push(
        shell,
        settings,
        SyncTrigger::ConflictResolution,
        CloudPushMode::ForceLocal,
        cx,
    );
}

/// 冲突处理：以远端目录整体替换本机；替换前仍写安全副本。
pub(crate) fn start_cloud_sync_force_pull(shell: &mut AppShell, cx: &mut Context<AppShell>) {
    let settings = app_settings(cx).sync.clone();
    if !settings.endpoint_enabled() {
        shell
            .state
            .set_error(rust_i18n::t!("sync.error_no_endpoint").to_string());
        cx.notify();
        return;
    }
    crate::cloud_sync::dispatch_cloud_sync_pull(
        shell,
        settings,
        SyncTrigger::ConflictResolution,
        CloudPullMode::ForceRemote,
        cx,
    );
}

/// 启动或定时触发的 Git 同步。
pub(crate) fn start_git_sync(
    shell: &mut AppShell,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) {
    let draft = shell.state.modals.settings.read(cx).state().clone();
    if let Err(error) = crate::settings::sync_form::persist_sync_credentials(
        &draft,
        shell.state.services.secret_store.as_ref(),
        window,
        cx,
    ) {
        shell.state.set_error(localized_error(&error));
        cx.notify();
        return;
    }
    let settings = crate::settings::sync_form::collect_sync_settings(&draft, cx);
    if !settings.git_enabled() {
        shell
            .state
            .set_error(rust_i18n::t!("sync.git_no_config").to_string());
        cx.notify();
        return;
    }
    update_app_settings(cx, |current| current.sync = settings.clone());
    let snapshot = app_settings(cx).clone();
    shell.state.persist_settings_reported(&snapshot);
    send_git_sync(shell, settings, SyncTrigger::Manual, cx);
}

/// 定时 Git 同步入口：秘密已经在钥匙串里，不再触碰设置表单。
pub(crate) fn start_git_sync_scheduled(shell: &mut AppShell, cx: &mut Context<AppShell>) {
    let settings = app_settings(cx).sync.clone();
    if settings.git_enabled() {
        send_git_sync(shell, settings, SyncTrigger::GitInterval, cx);
    }
}

fn send_git_sync(
    shell: &mut AppShell,
    settings: SyncSettings,
    trigger: SyncTrigger,
    cx: &mut Context<AppShell>,
) {
    let provider = settings
        .git
        .as_ref()
        .map_or_else(|| "git".to_string(), |_| "git".to_string());
    begin_sync_operation(
        shell,
        SyncOperation::Git,
        trigger,
        provider,
        rust_i18n::t!("sync.git_running").to_string(),
    );
    let catalog = shell.state.catalog.clone();
    if !shell.state.send_command(SyncCommand::GitSync {
        settings,
        catalog,
        trigger,
    }) {
        crate::cloud_sync::finish_sync_failure(
            shell,
            SyncOperation::Git,
            trigger,
            "git",
            rust_i18n::t!("sync.enqueue_failed").to_string(),
        );
    }
    cx.notify();
}

/// 使用设置表单现状准备一次手动动作，并把拓扑写入 Global 与设置文件。
fn manual_sync_settings(
    shell: &mut AppShell,
    window: &mut Window,
    cx: &mut Context<AppShell>,
) -> Option<SyncSettings> {
    let draft = shell.state.modals.settings.read(cx).state().clone();
    if let Err(error) = crate::settings::sync_form::persist_sync_credentials(
        &draft,
        shell.state.services.secret_store.as_ref(),
        window,
        cx,
    ) {
        shell.state.set_error(localized_error(&error));
        cx.notify();
        return None;
    }
    let settings = crate::settings::sync_form::collect_sync_settings(&draft, cx);
    if !settings.enabled {
        shell
            .state
            .set_error(rust_i18n::t!("sync.disabled_action").to_string());
        cx.notify();
        return None;
    }
    if !settings.has_endpoint() {
        shell
            .state
            .set_error(rust_i18n::t!("sync.error_no_endpoint").to_string());
        cx.notify();
        return None;
    }
    update_app_settings(cx, |current| current.sync = settings.clone());
    let snapshot = app_settings(cx).clone();
    shell.state.persist_settings_reported(&snapshot);
    Some(settings)
}

pub(crate) fn dispatch_cloud_sync_check(
    shell: &mut AppShell,
    settings: SyncSettings,
    trigger: SyncTrigger,
    cx: &mut Context<AppShell>,
) {
    let provider = provider_key(&settings);
    begin_sync_operation(
        shell,
        SyncOperation::Check,
        trigger,
        provider,
        rust_i18n::t!("sync.checking").to_string(),
    );
    let catalog = shell.state.catalog.clone();
    if !shell.state.send_command(SyncCommand::CloudSyncCheck {
        settings,
        catalog,
        trigger,
    }) {
        crate::cloud_sync::finish_sync_failure(
            shell,
            SyncOperation::Check,
            trigger,
            &provider_key(&app_settings(cx).sync),
            rust_i18n::t!("sync.enqueue_failed").to_string(),
        );
    }
    cx.notify();
}

pub(crate) fn dispatch_cloud_sync_test(
    shell: &mut AppShell,
    settings: SyncSettings,
    trigger: SyncTrigger,
    cx: &mut Context<AppShell>,
) {
    let provider = provider_key(&settings);
    begin_sync_operation(
        shell,
        SyncOperation::TestConnection,
        trigger,
        provider,
        rust_i18n::t!("sync.testing").to_string(),
    );
    if !shell
        .state
        .send_command(SyncCommand::CloudSyncTest { settings, trigger })
    {
        crate::cloud_sync::finish_sync_failure(
            shell,
            SyncOperation::TestConnection,
            trigger,
            &provider_key(&app_settings(cx).sync),
            rust_i18n::t!("sync.enqueue_failed").to_string(),
        );
    }
    cx.notify();
}

pub(crate) fn dispatch_cloud_sync_push(
    shell: &mut AppShell,
    settings: SyncSettings,
    trigger: SyncTrigger,
    mode: CloudPushMode,
    cx: &mut Context<AppShell>,
) {
    let provider = provider_key(&settings);
    begin_sync_operation(
        shell,
        SyncOperation::Push,
        trigger,
        provider,
        rust_i18n::t!("sync.push_started").to_string(),
    );
    let catalog = shell.state.catalog.clone();
    if !shell.state.send_command(SyncCommand::CloudSyncPush {
        settings,
        catalog,
        trigger,
        mode,
    }) {
        crate::cloud_sync::finish_sync_failure(
            shell,
            SyncOperation::Push,
            trigger,
            &provider_key(&app_settings(cx).sync),
            rust_i18n::t!("sync.enqueue_failed").to_string(),
        );
    }
    cx.notify();
}

pub(crate) fn dispatch_cloud_sync_pull(
    shell: &mut AppShell,
    settings: SyncSettings,
    trigger: SyncTrigger,
    mode: CloudPullMode,
    cx: &mut Context<AppShell>,
) {
    let provider = provider_key(&settings);
    begin_sync_operation(
        shell,
        SyncOperation::Pull,
        trigger,
        provider,
        rust_i18n::t!("sync.pull_started").to_string(),
    );
    if !shell.state.send_command(SyncCommand::CloudSyncPull {
        settings,
        trigger,
        mode,
    }) {
        crate::cloud_sync::finish_sync_failure(
            shell,
            SyncOperation::Pull,
            trigger,
            &provider_key(&app_settings(cx).sync),
            rust_i18n::t!("sync.enqueue_failed").to_string(),
        );
    }
    cx.notify();
}

fn begin_sync_operation(
    shell: &mut AppShell,
    operation: SyncOperation,
    trigger: SyncTrigger,
    _provider: String,
    message: String,
) {
    shell
        .state
        .cloud_sync
        .begin(operation, trigger, message.clone());
    shell.state.status = Some(message);
}

/// 记录一次失败并保留触发上下文；底层错误由事件处理层传入本地化文本。
pub(crate) fn finish_sync_failure(
    shell: &mut AppShell,
    operation: SyncOperation,
    trigger: SyncTrigger,
    provider: &str,
    message: String,
) {
    shell.state.set_error(message.clone());
    let entry = crate::cloud_sync::record_sync_history(
        shell,
        SyncHistoryRecord {
            operation,
            trigger,
            status: SyncHistoryStatus::Failed,
            provider,
            duration_ms: 0,
            sessions: 0,
            folders: 0,
            bytes: 0,
            message,
        },
    );
    shell.state.cloud_sync.finish(entry);
    shell.state.status = shell.state.cloud_sync.message.clone();
}

/// 写入内存历史并排队落盘。
pub(crate) fn record_sync_history(
    shell: &mut AppShell,
    record: SyncHistoryRecord<'_>,
) -> SyncHistoryEntry {
    let at_ms = chrono::Utc::now().timestamp_millis();
    let entry = SyncHistoryEntry {
        id: format!(
            "{at_ms}-{}-{}",
            record.operation.key(),
            record.trigger.key()
        ),
        at_ms,
        operation: record.operation,
        trigger: record.trigger,
        status: record.status,
        provider: record.provider.to_string(),
        duration_ms: record.duration_ms,
        sessions: record.sessions,
        folders: record.folders,
        bytes: record.bytes,
        message: record.message,
    };
    if let Some(writer) = &shell.state.sync_history_writer {
        writer.append(entry.clone());
    }
    entry
}

pub(crate) fn provider_key(settings: &SyncSettings) -> String {
    settings
        .active_endpoint()
        .map(|endpoint| endpoint.provider_key().to_string())
        .or_else(|| {
            (settings.active_channel == transom_core::SyncChannel::Git).then(|| "git".to_string())
        })
        .unwrap_or_else(|| "none".to_string())
}

/// 一次历史记录的参数集合，避免多字段平铺造成调用点顺序错误。
pub(crate) struct SyncHistoryRecord<'a> {
    pub(crate) operation: SyncOperation,
    pub(crate) trigger: SyncTrigger,
    pub(crate) status: SyncHistoryStatus,
    pub(crate) provider: &'a str,
    pub(crate) duration_ms: u64,
    pub(crate) sessions: usize,
    pub(crate) folders: usize,
    pub(crate) bytes: u64,
    pub(crate) message: String,
}
