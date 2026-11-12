//! 云同步表单的回填、汇总与凭据落盘。
//!
//! WebDAV 与 S3 各自保留一份配置；表单只编辑当前通道，但切换前已经填入的
//! 另一侧字段不会被清空。凭据按提供商写入独立钥匙串槽位。

use crate::app::app_settings;
use crate::cloud_sync::DEFAULT_SYNC_DIRECTORY;
use crate::settings::SettingsModalState;
use gpui_kit::component::input::InputState;
use gpui_kit::*;
use transom_core::{
    AppError, ErrorCode, GitSyncSettings, S3SyncSettings, SecretKey, SecretStore, SyncChannel,
    SyncSettings, WebdavSyncSettings,
};

/// 打开设置弹窗时回填云同步表单；秘密一律留空。
pub(crate) fn fill_sync_form(state: &SettingsModalState, window: &mut Window, cx: &mut App) {
    let sync = app_settings(cx).sync.clone();
    let form = &state.sync_form;
    let mut set = |input: &Entity<InputState>, value: &str| {
        input.update(cx, |state, cx| state.set_value(value, window, cx));
    };
    set(&form.device_name, &sync.device_name);
    set(
        &form.auto_pull_debounce,
        &sync.auto_push_debounce_seconds.to_string(),
    );
    set(&form.webdav_url, &sync.webdav.url);
    set(&form.webdav_username, &sync.webdav.username);
    set(&form.s3_endpoint, &sync.s3.endpoint);
    set(&form.s3_region, &sync.s3.region);
    set(&form.s3_bucket, &sync.s3.bucket);
    set(&form.s3_access_key, &sync.s3.access_key);
    let directory = sync
        .active_endpoint()
        .map(|endpoint| endpoint.directory().to_string())
        .unwrap_or_else(|| {
            if sync.webdav.directory.is_empty() && sync.s3.directory.is_empty() {
                DEFAULT_SYNC_DIRECTORY.to_string()
            } else if sync.active_channel == SyncChannel::S3 {
                sync.s3.directory.clone()
            } else {
                sync.webdav.directory.clone()
            }
        });
    set(&form.directory, &directory);
    if let Some(git) = &sync.git {
        set(&form.git_remote, &git.remote);
        set(&form.git_branch, &git.branch);
        set(&form.git_interval, &git.interval_minutes.to_string());
    } else {
        set(&form.git_branch, "transom-sync");
        set(&form.git_interval, "0");
    }
}

/// 从输入框汇总当前云同步配置（不落盘）。
pub(crate) fn collect_sync_settings(state: &SettingsModalState, cx: &App) -> SyncSettings {
    let form = &state.sync_form;
    let read = |input: &Entity<InputState>| input.read(cx).value().trim().to_string();
    let mut sync = app_settings(cx).sync.clone();
    let directory = read(&form.directory);
    match sync.active_channel {
        SyncChannel::Webdav => {
            sync.webdav = WebdavSyncSettings {
                url: read(&form.webdav_url),
                username: read(&form.webdav_username),
                directory,
            };
        }
        SyncChannel::S3 => {
            sync.s3 = S3SyncSettings {
                endpoint: read(&form.s3_endpoint),
                region: read(&form.s3_region),
                bucket: read(&form.s3_bucket),
                access_key: read(&form.s3_access_key),
                directory,
                virtual_host_style: sync.s3.virtual_host_style,
            };
        }
        SyncChannel::Off | SyncChannel::Git => {}
    }
    sync.device_name = read(&form.device_name);
    sync.auto_push_debounce_seconds = read(&form.auto_pull_debounce)
        .parse::<u64>()
        .unwrap_or(sync.auto_push_debounce_seconds)
        .clamp(5, 3600);
    let git_remote = read(&form.git_remote);
    sync.git = if git_remote.is_empty() {
        None
    } else {
        Some(
            GitSyncSettings {
                remote: git_remote,
                branch: read(&form.git_branch),
                interval_minutes: read(&form.git_interval).parse::<u64>().unwrap_or(0),
            }
            .sanitized(),
        )
        .flatten()
    };
    sync.normalize();
    sync
}

/// 把非空的凭据输入写入钥匙串并清空输入框。
///
/// WebDAV 密码、S3 Secret Key 与 S3 Session Token 使用独立槽位；切换通道后
/// 再切回来仍能复用原凭据。
///
/// # Errors
/// 口令过短或系统钥匙串写入失败时返回领域错误，调用方应中止本次动作。
pub(crate) fn persist_sync_credentials(
    state: &SettingsModalState,
    secret_store: &dyn SecretStore,
    window: &mut Window,
    cx: &mut App,
) -> Result<(), AppError> {
    let form = &state.sync_form;
    let active = app_settings(cx).sync.active_channel;
    if active == SyncChannel::Webdav {
        let credential = form.webdav_password.read(cx).value().trim().to_string();
        if !credential.is_empty() {
            secret_store.set(&SecretKey::cloud_sync_webdav_credential(), &credential)?;
            form.webdav_password
                .update(cx, |state, cx| state.set_value("", window, cx));
        }
    }
    if active == SyncChannel::S3 {
        let credential = form.s3_secret.read(cx).value().trim().to_string();
        if !credential.is_empty() {
            secret_store.set(&SecretKey::cloud_sync_s3_credential(), &credential)?;
            form.s3_secret
                .update(cx, |state, cx| state.set_value("", window, cx));
        }
        let session_token = form.s3_session_token.read(cx).value().trim().to_string();
        if !session_token.is_empty() {
            secret_store.set(&SecretKey::cloud_sync_s3_session_token(), &session_token)?;
            form.s3_session_token
                .update(cx, |state, cx| state.set_value("", window, cx));
        }
    }
    let git_token = form.git_token.read(cx).value().trim().to_string();
    if !git_token.is_empty() {
        secret_store.set(&SecretKey::git_sync_token(), &git_token)?;
        form.git_token
            .update(cx, |state, cx| state.set_value("", window, cx));
    }
    let passphrase = form.passphrase.read(cx).value().trim().to_string();
    if !passphrase.is_empty() {
        if passphrase.chars().count() < 8 {
            return Err(AppError::with_detail(
                ErrorCode::Configuration,
                rust_i18n::t!("backup.error_short").to_string(),
            ));
        }
        secret_store.set(&SecretKey::cloud_sync_passphrase(), &passphrase)?;
        form.passphrase
            .update(cx, |state, cx| state.set_value("", window, cx));
    }
    Ok(())
}
