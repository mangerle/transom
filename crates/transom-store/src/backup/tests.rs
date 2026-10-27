//! 会话目录备份与钥匙串口令导入导出单元测试。

use super::*;
use tempfile::tempdir;
use transom_core::{AuthMethod, Connection, SecretKind, SecretOwner, Session, SshConnection};
use zeroize::Zeroizing;

/// 构造一个仅含内存钥匙串的假存储，避免测试依赖系统 Secret Service。
#[derive(Default)]
struct FakeSecretStore {
    entries: parking_lot::Mutex<std::collections::HashMap<String, String>>,
    available: parking_lot::Mutex<bool>,
}

impl FakeSecretStore {
    fn available() -> Self {
        Self {
            entries: parking_lot::Mutex::new(std::collections::HashMap::new()),
            available: parking_lot::Mutex::new(true),
        }
    }

    fn unavailable() -> Self {
        Self {
            entries: parking_lot::Mutex::new(std::collections::HashMap::new()),
            available: parking_lot::Mutex::new(false),
        }
    }

    fn put(&self, key: &SecretKey, value: &str) {
        self.entries.lock().insert(key.account(), value.to_string());
    }
}

impl SecretStore for FakeSecretStore {
    fn get(&self, key: &SecretKey) -> Result<Option<Zeroizing<String>>, AppError> {
        if !*self.available.lock() {
            return Err(AppError::with_detail(
                ErrorCode::SecretStoreUnavailable,
                "系统钥匙串不可用",
            ));
        }
        Ok(self
            .entries
            .lock()
            .get(&key.account())
            .map(|value| Zeroizing::new(value.clone())))
    }

    fn set(&self, key: &SecretKey, secret: &str) -> Result<(), AppError> {
        if !*self.available.lock() {
            return Err(AppError::with_detail(
                ErrorCode::SecretStoreUnavailable,
                "系统钥匙串不可用",
            ));
        }
        self.entries
            .lock()
            .insert(key.account(), secret.to_string());
        Ok(())
    }

    fn delete(&self, key: &SecretKey) -> Result<(), AppError> {
        self.entries.lock().remove(&key.account());
        Ok(())
    }
}

fn catalog() -> SessionCatalog {
    let mut catalog = SessionCatalog::default();
    let folder = catalog.ensure_folder_path("生产/数据库").unwrap();
    let mut session = Session::new(
        "db-01",
        Connection::Ssh(SshConnection {
            host: "db.internal.example.com".into(),
            port: 22,
            username: "deploy".into(),
            auth: AuthMethod::Password { save: true },
            options: Default::default(),
            proxy_jump: None,
        }),
    );
    session.folder_id = Some(folder);
    catalog.sessions.push(session);
    catalog
}

fn service(store: Arc<dyn SecretStore>) -> CatalogBackupService {
    CatalogBackupService::new(store)
}

#[test]
fn encrypted_round_trip_for_both_formats() {
    let document = BackupDocument::from_catalog(&catalog());
    for format in [BackupFormat::Json, BackupFormat::Yaml] {
        let encrypted = encrypt_document(&document, "correct horse", format).unwrap();
        let text = String::from_utf8_lossy(&encrypted);
        assert!(!text.contains("db.internal.example.com"));
        let restored = decrypt_document(&encrypted, "correct horse").unwrap();
        assert_eq!(restored.sessions.len(), 1);
        assert_eq!(restored.folders.len(), 2);
        assert_eq!(
            restored.sessions[0].folder_id,
            restored.folders.last().map(|folder| folder.id)
        );
    }
}

#[test]
fn plaintext_export_is_readable_json_without_envelope() {
    let document = BackupDocument::from_catalog(&catalog());
    let data = serialize_document(&document, BackupFormat::Json).unwrap();
    let text = String::from_utf8_lossy(&data);
    assert!(text.contains("db.internal.example.com"));
    assert!(!text.contains("transom-backup"));
    let parsed: BackupDocument = serde_json::from_slice(&data).unwrap();
    assert_eq!(parsed.sessions.len(), 1);
    assert!(parsed.secrets.is_none());
}

#[test]
fn plain_file_is_rejected_as_encrypted_and_vice_versa() {
    let document = BackupDocument::from_catalog(&catalog());
    let plain = serialize_document(&document, BackupFormat::Json).unwrap();

    let error = read_backup(Path::new("/nonexistent"), None).unwrap_err();
    assert_eq!(error.code, ErrorCode::Io);

    let dir = tempdir().unwrap();
    let plain_path = dir.path().join("plain.json");
    fs::write(&plain_path, &plain).unwrap();
    let error = read_backup(&plain_path, Some("secret")).unwrap_err();
    assert_eq!(error.code, ErrorCode::BackupNotEncrypted);

    let encrypted_path = dir.path().join("secret.json");
    write_backup(
        &encrypted_path,
        &document,
        Some("secret"),
        BackupFormat::Json,
    )
    .unwrap();
    let error = read_backup(&encrypted_path, None).unwrap_err();
    assert_eq!(error.code, ErrorCode::BackupEncrypted);
}

#[test]
fn wrong_passphrase_and_tampering_are_rejected() {
    let document = BackupDocument::from_catalog(&catalog());
    let encrypted = encrypt_document(&document, "secret", BackupFormat::Json).unwrap();
    assert!(decrypt_document(&encrypted, "wrong").is_err());

    let mut tampered = encrypted.clone();
    let last = tampered.len() - 2;
    tampered[last] = if tampered[last] == b'A' { b'B' } else { b'A' };
    assert!(decrypt_document(&tampered, "secret").is_err());
}

#[test]
fn file_round_trip_and_format_detection() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("backup.yaml");
    assert_eq!(BackupFormat::from_path(&path), BackupFormat::Yaml);
    let document = BackupDocument::from_catalog(&catalog());
    write_backup(&path, &document, Some("passphrase"), BackupFormat::Yaml).unwrap();
    let loaded = read_backup(&path, Some("passphrase")).unwrap();
    assert_eq!(loaded.sessions.len(), 1);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}

#[test]
fn empty_passphrase_is_rejected() {
    let document = BackupDocument::from_catalog(&catalog());
    assert!(encrypt_document(&document, "", BackupFormat::Json).is_err());
}

#[test]
fn plaintext_export_with_secrets_is_rejected() {
    let store = Arc::new(FakeSecretStore::available());
    let catalog = catalog();
    let key = SecretKey::new(catalog.sessions[0].id, SecretKind::Password);
    store.put(&key, "s3cret");
    let service = service(store);

    let dir = tempdir().unwrap();
    let path = dir.path().join("plain-with-secrets.json");
    let options = BackupExportOptions {
        include_secrets: true,
        ..Default::default()
    };
    let error = service.export(&path, &catalog, &options).unwrap_err();
    assert_eq!(error.code, ErrorCode::Configuration);
    assert!(!path.exists(), "拒绝导出时不得留下半成品文件");
}

#[test]
fn export_includes_secrets_only_when_requested_and_encrypted() {
    let store = Arc::new(FakeSecretStore::available());
    let catalog = catalog();
    let key = SecretKey::new(catalog.sessions[0].id, SecretKind::Password);
    store.put(&key, "s3cret");
    let service = service(store.clone());

    let dir = tempdir().unwrap();
    let path = dir.path().join("with-secrets.json");
    let options = BackupExportOptions {
        include_secrets: true,
        passphrase: Some(Zeroizing::new("correct horse".to_string())),
    };
    let report = service
        .export(&path, &catalog, &options)
        .expect("加密导出应当成功");
    assert_eq!(report.secrets, 1);
    assert!(report.encrypted);

    let text = fs::read_to_string(&path).unwrap();
    assert!(!text.contains("s3cret"), "加密后不得出现明文口令");
    let loaded = read_backup(&path, Some("correct horse")).unwrap();
    assert_eq!(
        loaded.secrets.as_ref().map(Vec::len),
        Some(1),
        "解密后应恢复一条口令"
    );

    let plain_path = dir.path().join("without-secrets.json");
    let report = service
        .export(&plain_path, &catalog, &BackupExportOptions::default())
        .expect("导出应当成功");
    assert_eq!(report.secrets, 0);
    let text = fs::read_to_string(&plain_path).unwrap();
    assert!(!text.contains("s3cret"));
}

#[test]
fn export_fails_when_keyring_unavailable() {
    let service = service(Arc::new(FakeSecretStore::unavailable()));
    let dir = tempdir().unwrap();
    let path = dir.path().join("broken.json");
    let options = BackupExportOptions {
        include_secrets: true,
        passphrase: Some(Zeroizing::new("correct horse".to_string())),
    };
    let error = service.export(&path, &catalog(), &options).unwrap_err();
    assert_eq!(error.code, ErrorCode::SecretStoreUnavailable);
    assert!(!path.exists());
}

#[test]
fn restore_keeps_existing_secret() {
    let store = Arc::new(FakeSecretStore::available());
    let catalog = catalog();
    let key = SecretKey::new(catalog.sessions[0].id, SecretKind::Password);
    store.put(&key, "local");

    let service = service(store.clone());
    let secrets = vec![BackupSecret {
        owner: SecretOwner::Session(catalog.sessions[0].id),
        kind: SecretKind::Password,
        value: Zeroizing::new("from-backup".to_string()),
    }];
    let report = service.restore_secrets(&secrets).unwrap();
    assert_eq!(report.kept, 1);
    assert_eq!(report.restored, 0);
    let stored = store.get(&key).unwrap().unwrap();
    assert_eq!(stored.as_str(), "local");
}

#[test]
fn restore_writes_missing_secret() {
    let store = Arc::new(FakeSecretStore::available());
    let catalog = catalog();
    let key = SecretKey::new(catalog.sessions[0].id, SecretKind::Password);

    let service = service(store.clone());
    let secrets = vec![BackupSecret {
        owner: SecretOwner::Session(catalog.sessions[0].id),
        kind: SecretKind::Password,
        value: Zeroizing::new("from-backup".to_string()),
    }];
    let report = service.restore_secrets(&secrets).unwrap();
    assert_eq!(report.restored, 1);
    let stored = store.get(&key).unwrap().unwrap();
    assert_eq!(stored.as_str(), "from-backup");
}
