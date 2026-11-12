use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use transom_core::{
    AppError, BackupDocument, CloudPullMode, CloudPushMode, CloudSyncTarget, ErrorCode,
    S3SyncSettings, SecretKey, SecretStore, Session, SessionCatalog, SyncChannel, SyncEndpoint,
    SyncSettings, SyncTrigger, WebdavSyncSettings, collect_catalog_secrets,
};
use zeroize::Zeroizing;

use super::{CloudSyncService, build_envelope, bytes_hash, resolve_relation};

#[derive(Default)]
struct MemorySecrets {
    values: Mutex<HashMap<String, Zeroizing<String>>>,
}

impl SecretStore for MemorySecrets {
    fn get(&self, key: &SecretKey) -> Result<Option<Zeroizing<String>>, AppError> {
        Ok(self.values.lock().unwrap().get(&key.account()).cloned())
    }

    fn set(&self, key: &SecretKey, secret: &str) -> Result<(), AppError> {
        self.values
            .lock()
            .unwrap()
            .insert(key.account(), Zeroizing::new(secret.to_string()));
        Ok(())
    }

    fn delete(&self, key: &SecretKey) -> Result<(), AppError> {
        self.values.lock().unwrap().remove(&key.account());
        Ok(())
    }
}

#[derive(Default)]
struct MemoryTarget {
    stored: Mutex<Option<Vec<u8>>>,
}

#[async_trait]
impl CloudSyncTarget for MemoryTarget {
    async fn put_document(&self, data: &[u8]) -> Result<(), AppError> {
        *self.stored.lock().unwrap() = Some(data.to_vec());
        Ok(())
    }

    async fn get_document(&self) -> Result<Vec<u8>, AppError> {
        self.stored
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| AppError::with_detail(ErrorCode::CloudSyncMissing, "远端还没有同步备份"))
    }
}

fn catalog() -> SessionCatalog {
    let mut catalog = SessionCatalog::default();
    catalog.sessions.push(Session::new(
        "web-01",
        transom_core::Connection::Local(Default::default()),
    ));
    catalog
}

fn settings(endpoint: Option<SyncEndpoint>) -> SyncSettings {
    let mut settings = SyncSettings {
        enabled: true,
        device_id: "device-test".into(),
        include_secrets: true,
        ..Default::default()
    };
    match endpoint {
        Some(SyncEndpoint::Webdav {
            url,
            username,
            directory,
        }) => {
            settings.active_channel = SyncChannel::Webdav;
            settings.webdav = WebdavSyncSettings {
                url,
                username,
                directory,
            };
        }
        Some(SyncEndpoint::S3 {
            endpoint,
            region,
            bucket,
            access_key,
            directory,
            virtual_host_style,
        }) => {
            settings.active_channel = SyncChannel::S3;
            settings.s3 = S3SyncSettings {
                endpoint,
                region,
                bucket,
                access_key,
                directory,
                virtual_host_style,
            };
        }
        None => {}
    }
    settings
}

fn webdav_endpoint() -> SyncEndpoint {
    SyncEndpoint::Webdav {
        url: "https://dav.example.com/dav".into(),
        username: "user".into(),
        directory: "transom".into(),
    }
}

#[test]
fn git_sync_requires_dedicated_token() {
    let secrets = MemorySecrets::default();
    secrets
        .set(
            &SecretKey::cloud_sync_webdav_credential(),
            "webdav-password",
        )
        .unwrap();
    secrets
        .set(&SecretKey::cloud_sync_passphrase(), "sync-passphrase")
        .unwrap();
    let service = CloudSyncService::new(Arc::new(secrets)).unwrap();
    let mut config = settings(None);
    config.git = Some(transom_core::GitSyncSettings {
        remote: "https://git.example.com/repo.git".into(),
        ..Default::default()
    });
    let cache = tempfile::tempdir().unwrap();

    let error = service
        .run_git_sync(&config, &catalog(), cache.path())
        .expect_err("缺少 Git 专用令牌必须拒绝");
    assert_eq!(error.code, ErrorCode::Configuration);
    assert!(error.detail.unwrap_or_default().contains("单独"));
}

#[tokio::test]
async fn push_requires_endpoint_and_passphrase() {
    let service = CloudSyncService::new(Arc::new(MemorySecrets::default())).unwrap();

    let error = service
        .push(&catalog(), &settings(None), CloudPushMode::Merge)
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Configuration);
    assert!(error.detail.unwrap_or_default().contains("端点"));

    let error = service
        .push(
            &catalog(),
            &settings(Some(webdav_endpoint())),
            CloudPushMode::Merge,
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Configuration);
    assert!(error.detail.unwrap_or_default().contains("同步口令"));
}

#[tokio::test]
async fn envelope_round_trips_and_rejects_wrong_passphrase() {
    let secrets = MemorySecrets::default();
    secrets
        .set(&SecretKey::cloud_sync_webdav_credential(), "app-password")
        .unwrap();

    let catalog = catalog();
    let (data, secret_count) = build_envelope(&catalog, true, &secrets, "correct horse").unwrap();
    assert_eq!(secret_count, 0, "测试目录未保存任何会话口令");
    assert!(!String::from_utf8_lossy(&data).contains("web-01"));

    let document = transom_crypto::backup::decrypt_document(&data, "correct horse").unwrap();
    assert_eq!(document.sessions.len(), 1);
    assert!(transom_crypto::backup::decrypt_document(&data, "wrong").is_err());
}

#[tokio::test]
async fn empty_remote_reports_missing() {
    let remote = MemoryTarget::default();
    let error = remote.get_document().await.unwrap_err();
    assert_eq!(error.code, ErrorCode::CloudSyncMissing);
}

#[test]
fn catalog_hash_is_stable_and_content_sensitive() {
    let first = catalog();
    assert_eq!(
        transom_core::sync_catalog_hash(&first).unwrap(),
        transom_core::sync_catalog_hash(&first).unwrap()
    );
    let mut second = first.clone();
    second.sessions[0].name = "changed".into();
    assert_ne!(
        transom_core::sync_catalog_hash(&first).unwrap(),
        transom_core::sync_catalog_hash(&second).unwrap()
    );
    assert_eq!(bytes_hash(b"a"), bytes_hash(b"a"));
}

#[test]
fn relation_detects_up_to_date_local_ahead_remote_ahead_and_divergence() {
    let local = catalog();
    let local_hash = transom_core::sync_catalog_hash(&local).unwrap();
    let summary = transom_core::SyncDocumentSummary::from_catalog(&local);
    let mut settings = settings(Some(webdav_endpoint()));
    settings.baselines.insert(
        "webdav".into(),
        transom_core::SyncBaseline {
            last_sync_catalog_hash: Some(local_hash.clone()),
            last_remote_hash: Some("remote-a".into()),
            ..Default::default()
        },
    );
    let (local_changed, remote_changed, relation) = resolve_relation(
        &settings,
        &local_hash,
        &summary,
        Some("remote-a"),
        Some(&summary),
    );
    assert!(!local_changed && !remote_changed);
    assert_eq!(relation, transom_core::SyncRelation::UpToDate);

    let (_, _, relation) = resolve_relation(
        &settings,
        "local-b",
        &summary,
        Some("remote-a"),
        Some(&summary),
    );
    assert_eq!(relation, transom_core::SyncRelation::LocalAhead);
    let (_, _, relation) = resolve_relation(
        &settings,
        &local_hash,
        &summary,
        Some("remote-b"),
        Some(&summary),
    );
    assert_eq!(relation, transom_core::SyncRelation::RemoteAhead);
    let (_, _, relation) = resolve_relation(
        &settings,
        "local-b",
        &summary,
        Some("remote-b"),
        Some(&summary),
    );
    assert_eq!(relation, transom_core::SyncRelation::Diverged);
}

#[test]
fn pull_mode_values_are_distinct_contracts() {
    assert_ne!(CloudPushMode::Merge, CloudPushMode::ForceLocal);
    assert_ne!(CloudPullMode::Merge, CloudPullMode::ForceRemote);
    assert_ne!(SyncTrigger::Manual, SyncTrigger::Periodic);
    let document = BackupDocument::from_catalog(&catalog());
    assert_eq!(document.sessions.len(), 1);
    assert_eq!(
        collect_catalog_secrets(&MemorySecrets::default(), &catalog())
            .unwrap()
            .len(),
        0
    );
}
