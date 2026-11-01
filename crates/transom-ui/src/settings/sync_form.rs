//! 云同步表单的回填、汇总与凭据落盘。
//!
//! 设计原理（Why）：这些逻辑只读写 `SettingsModalState::sync_form` 与全局
//! 同步配置，不依赖外壳本身。抽成接收 `&SettingsModalState` 的自由函数后，
//! 设置实体（保存流程）与外壳（立即上传/下载）共用同一份实现，不再各留一份
//! 会漂移的副本。
//!
//! 代价与局限：凭据写入需要窗口句柄清空输入框，因此函数签名仍带
//! `&mut Window`；密钥串失败以 `AppError` 返回，由调用方决定提示位置。

use crate::app::app_settings;
use crate::cloud_sync::DEFAULT_SYNC_DIRECTORY;
use crate::settings::SettingsModalState;
use gpui_kit::component::input::InputState;
use gpui_kit::*;
use transom_core::{
    AppError, ErrorCode, GitSyncSettings, SecretKey, SecretStore, SyncEndpoint, SyncSettings,
};

/// 打开设置弹窗时回填云同步表单；秘密一律留空。
pub(crate) fn fill_sync_form(state: &SettingsModalState, window: &mut Window, cx: &mut App) {
    let sync = app_settings(cx).sync.clone();
    let form = &state.sync_form;
    let mut set = |input: &Entity<InputState>, value: &str| {
        input.update(cx, |state, cx| state.set_value(value, window, cx));
    };
    let directory = || {
        sync.endpoint
            .as_ref()
            .map(|endpoint| endpoint.directory().to_string())
            .unwrap_or_else(|| DEFAULT_SYNC_DIRECTORY.to_string())
    };
    if let Some(git) = &sync.git {
        set(&form.git_remote, &git.remote);
        set(&form.git_branch, &git.branch);
        set(&form.git_interval, &git.interval_minutes.to_string());
    } else {
        set(&form.git_branch, "transom-sync");
        set(&form.git_interval, "0");
    }
    match &sync.endpoint {
        Some(SyncEndpoint::Webdav {
            url,
            username,
            directory,
        }) => {
            set(&form.webdav_url, url);
            set(&form.webdav_username, username);
            set(&form.directory, directory);
        }
        Some(SyncEndpoint::S3 {
            endpoint,
            region,
            bucket,
            access_key,
            directory,
        }) => {
            set(&form.s3_endpoint, endpoint);
            set(&form.s3_region, region);
            set(&form.s3_bucket, bucket);
            set(&form.s3_access_key, access_key);
            set(&form.directory, directory);
        }
        None => {
            set(&form.directory, &directory());
        }
    }
}

/// 从输入框汇总当前云同步配置（不落盘）。
///
/// 端点类型以设置内存态为准（用户在页上点选），字段值取输入框现状，
/// 因此“填完直接点上传”无需先保存；汇总后再做一次清洗兜底。
pub(crate) fn collect_sync_settings(state: &SettingsModalState, cx: &App) -> SyncSettings {
    let form = &state.sync_form;
    let read = |input: &Entity<InputState>| input.read(cx).value().trim().to_string();
    let endpoint = match app_settings(cx).sync.endpoint.clone() {
        Some(SyncEndpoint::Webdav { .. }) => Some(SyncEndpoint::Webdav {
            url: read(&form.webdav_url),
            username: read(&form.webdav_username),
            directory: read(&form.directory),
        }),
        Some(SyncEndpoint::S3 { .. }) => Some(SyncEndpoint::S3 {
            endpoint: read(&form.s3_endpoint),
            region: read(&form.s3_region),
            bucket: read(&form.s3_bucket),
            access_key: read(&form.s3_access_key),
            directory: read(&form.directory),
        }),
        None => None,
    };
    let mut sync = app_settings(cx).sync.clone();
    sync.endpoint = endpoint.and_then(SyncEndpoint::sanitized);
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
    sync
}

/// 把非空的凭据输入写入钥匙串并清空输入框。
///
/// 只保存与当前端点类型匹配的凭据：WebDAV 密码与 S3 Secret Key 共用
/// 同一槽位（同一时刻只有一个端点生效），同时写两个会互相覆盖。
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
    let is_webdav = matches!(
        app_settings(cx).sync.endpoint,
        Some(SyncEndpoint::Webdav { .. })
    );
    let credential = if is_webdav {
        form.webdav_password.read(cx).value().trim().to_string()
    } else {
        form.s3_secret.read(cx).value().trim().to_string()
    };
    let passphrase = form.passphrase.read(cx).value().trim().to_string();

    if !credential.is_empty() {
        let key = SecretKey::cloud_sync_credential();
        secret_store.set(&key, &credential)?;
        let cleared = if is_webdav {
            &form.webdav_password
        } else {
            &form.s3_secret
        };
        cleared.update(cx, |state, cx| state.set_value("", window, cx));
    }
    let git_token = form.git_token.read(cx).value().trim().to_string();
    if !git_token.is_empty() {
        let key = SecretKey::git_sync_token();
        secret_store.set(&key, &git_token)?;
        form.git_token
            .update(cx, |state, cx| state.set_value("", window, cx));
    }
    if !passphrase.is_empty() {
        if passphrase.chars().count() < 8 {
            return Err(AppError::with_detail(
                ErrorCode::Configuration,
                rust_i18n::t!("backup.error_short").to_string(),
            ));
        }
        let key = SecretKey::cloud_sync_passphrase();
        secret_store.set(&key, &passphrase)?;
        form.passphrase
            .update(cx, |state, cx| state.set_value("", window, cx));
    }
    Ok(())
}
