//! 云同步服务内部辅助：目标构造、凭据读取、信封构建与关系判定。

use std::time::Instant;

use sha2::{Digest, Sha256};
use transom_core::{
    AppError, BackupDocument, CloudSyncTarget, ErrorCode, SecretKey, SecretStore, SessionCatalog,
    SyncDocumentSummary, SyncEndpoint, SyncRelation, SyncSettings, collect_catalog_secrets,
};
use transom_crypto::backup::{BackupFormat, encrypt_document};
use zeroize::Zeroizing;

use crate::s3::S3Target;
use crate::webdav::WebDavTarget;

/// 按端点配置构造远端目标；凭据在调用方的阻塞线程内读取。
pub(super) fn build_target(
    http: &reqwest::Client,
    secrets: &dyn SecretStore,
    endpoint: &SyncEndpoint,
) -> Result<Box<dyn CloudSyncTarget>, AppError> {
    match endpoint {
        SyncEndpoint::Webdav {
            url,
            username,
            directory,
        } => {
            let password = read_secret_with_legacy(
                secrets,
                &SecretKey::cloud_sync_webdav_credential(),
                &SecretKey::cloud_sync_credential(),
            )?;
            Ok(Box::new(WebDavTarget::new(
                http.clone(),
                url,
                username,
                password.as_deref().map(String::as_str),
                directory,
            )?))
        }
        SyncEndpoint::S3 {
            endpoint,
            region,
            bucket,
            access_key,
            directory,
            virtual_host_style,
        } => {
            let secret_key = read_secret_with_legacy(
                secrets,
                &SecretKey::cloud_sync_s3_credential(),
                &SecretKey::cloud_sync_credential(),
            )?
            .ok_or_else(|| {
                AppError::with_detail(
                    ErrorCode::Configuration,
                    "尚未在钥匙串保存 S3 Secret Access Key，请先在设置中填写并保存",
                )
            })?;
            let session_token = secrets.get(&SecretKey::cloud_sync_s3_session_token())?;
            Ok(Box::new(S3Target::new(
                http.clone(),
                crate::s3::S3TargetOptions {
                    endpoint,
                    region,
                    bucket,
                    access_key,
                    secret_key,
                    session_token,
                    directory,
                    virtual_host_style: *virtual_host_style,
                },
            )?))
        }
    }
}

/// 读取提供商专属凭据；旧安装回退到共享槽位。
pub(super) fn read_secret_with_legacy(
    secrets: &dyn SecretStore,
    current: &SecretKey,
    legacy: &SecretKey,
) -> Result<Option<Zeroizing<String>>, AppError> {
    match secrets.get(current)? {
        Some(value) => Ok(Some(value)),
        None => secrets.get(legacy),
    }
}

pub(super) fn passphrase_from(secrets: &dyn SecretStore) -> Result<Zeroizing<String>, AppError> {
    secrets
        .get(&SecretKey::cloud_sync_passphrase())?
        .ok_or_else(|| {
            AppError::with_detail(
                ErrorCode::Configuration,
                "尚未设置同步口令，云端备份必须加密；请先在设置中填写并保存",
            )
        })
}

/// 在阻塞线程池执行钥匙串等同步操作，并统一转换任务异常。
pub(super) async fn run_blocking<T, F>(action: &str, job: F) -> Result<T, AppError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, AppError> + Send + 'static,
{
    match tokio::task::spawn_blocking(job).await {
        Ok(result) => result,
        Err(error) => Err(AppError::with_detail(
            ErrorCode::CloudSyncFailed,
            format!("{action}任务异常终止: {error}"),
        )),
    }
}

/// 取出当前通道对应的 WebDAV/S3 端点。
pub(super) fn required_endpoint(settings: &SyncSettings) -> Result<SyncEndpoint, AppError> {
    settings
        .active_endpoint()
        .ok_or_else(|| AppError::with_detail(ErrorCode::Configuration, "尚未配置云同步端点"))
}

/// 构建加密信封：收集口令（可选）-> 快照文档 -> Argon2id 派生加密。
///
/// 返回信封字节与加密口令数；阻塞操作，调用方必须放在阻塞线程池执行。
pub(super) fn build_envelope(
    catalog: &SessionCatalog,
    include_secrets: bool,
    secrets: &dyn SecretStore,
    passphrase: &str,
) -> Result<(Vec<u8>, usize), AppError> {
    let mut document = BackupDocument::from_catalog(catalog);
    let mut secret_count = 0;
    if include_secrets {
        let collected = collect_catalog_secrets(secrets, catalog)?;
        secret_count = collected.len();
        document.secrets = Some(collected);
    }
    let data = encrypt_document(&document, passphrase, BackupFormat::Json)?;
    Ok((data, secret_count))
}

pub(super) fn bytes_hash(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

pub(super) fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

pub(super) fn resolve_relation(
    settings: &SyncSettings,
    local_hash: &str,
    local_summary: &SyncDocumentSummary,
    remote_hash: Option<&str>,
    remote_summary: Option<&SyncDocumentSummary>,
) -> (bool, bool, SyncRelation) {
    let has_local_content = local_summary.sessions > 0
        || local_summary.folders > 0
        || local_summary.identities > 0
        || local_summary.engines > 0
        || local_summary.engine_folders > 0;
    let baseline = settings.active_baseline();
    let local_changed = baseline
        .last_sync_catalog_hash
        .as_deref()
        .map_or(has_local_content, |value| value != local_hash);
    let remote_present = remote_summary.is_some();
    let remote_changed = if remote_present {
        baseline
            .last_remote_hash
            .as_deref()
            .is_none_or(|value| Some(value) != remote_hash)
    } else {
        baseline.last_remote_hash.is_some()
    };
    let relation = if !remote_present {
        if baseline.last_remote_hash.is_some() {
            SyncRelation::RemoteMissing
        } else if local_changed {
            SyncRelation::LocalAhead
        } else {
            SyncRelation::UpToDate
        }
    } else if !local_changed && !remote_changed {
        SyncRelation::UpToDate
    } else if local_changed && remote_changed {
        SyncRelation::Diverged
    } else if local_changed {
        SyncRelation::LocalAhead
    } else {
        SyncRelation::RemoteAhead
    };
    (local_changed, remote_changed, relation)
}
