//! 会话目录的备份文件读写：文件形态、钥匙串口令出入口与格式探测。
//!
//! 加解密（Argon2id 派生 + XChaCha20-Poly1305 信封）位于
//! [`transom_core::crypto::backup`]：它同时被云同步复用，上提到契约层后
//! 网络层不再依赖本存储层。本文件只负责"文件"：读写、体积上限、原子落盘
//! 与钥匙串口令的导出/导入。
//!
//! 安全设计：
//! - 明文备份默认不含系统钥匙串里的密码/口令，只有用户显式勾选才会写入，
//!   且包含口令时必须加密（见 `BackupService::export` 的入口校验）；
//! - 文件以 0600 权限原子写入，导入前限制文件大小，防止恶意超大文件。

use std::fs;
use std::path::Path;
use std::sync::Arc;
use transom_core::crypto::backup::{
    BackupFormat, MAX_BACKUP_BYTES, decrypt_document, encrypt_document, is_encrypted_envelope,
};
use transom_core::{
    AppError, BackupDocument, BackupExportOptions, BackupExportReport, BackupImportOptions,
    BackupSecret, BackupService, ErrorCode, SecretKey, SecretRestoreReport, SecretStore,
    SessionCatalog, collect_catalog_secrets,
};

/// 会话目录备份服务：文件格式、加密与钥匙串口令的出入口。
///
/// 口令的读写放在这一层，是因为只有这里同时握有"备份内容"与"系统钥匙串"
/// 两个知识面：导出时按会话的认证方式取口令，导入时按"本机已存在则保留"
/// 的规则写回。
pub struct CatalogBackupService {
    secrets: Arc<dyn SecretStore>,
}

impl CatalogBackupService {
    pub fn new(secrets: Arc<dyn SecretStore>) -> Self {
        Self { secrets }
    }
}

impl BackupService for CatalogBackupService {
    fn export(
        &self,
        path: &Path,
        catalog: &SessionCatalog,
        options: &BackupExportOptions,
    ) -> Result<BackupExportReport, AppError> {
        // 口令明文落盘是不可接受的：包含口令就必须加密。UI 会联动开关，
        // 但这里仍要拒绝，防止其它调用方或未来入口绕过界面约束。
        if options.include_secrets && options.passphrase.is_none() {
            return Err(AppError::with_detail(
                ErrorCode::Configuration,
                "包含会话口令的备份必须加密；请设置备份口令，或取消包含口令",
            ));
        }
        let mut document = BackupDocument::from_catalog(catalog);
        if options.include_secrets {
            document.secrets = Some(collect_catalog_secrets(self.secrets.as_ref(), catalog)?);
        }
        let format = BackupFormat::from_path(path);
        write_backup(
            path,
            &document,
            options
                .passphrase
                .as_ref()
                .map(|passphrase| passphrase.as_str()),
            format,
        )?;
        Ok(BackupExportReport {
            sessions: catalog.sessions.len(),
            folders: catalog.folders.len(),
            secrets: document.secrets.as_ref().map_or(0, |secrets| secrets.len()),
            encrypted: options.passphrase.is_some(),
        })
    }

    fn import(
        &self,
        path: &Path,
        options: &BackupImportOptions,
    ) -> Result<BackupDocument, AppError> {
        read_backup(
            path,
            options
                .passphrase
                .as_ref()
                .map(|passphrase| passphrase.as_str()),
        )
    }

    fn restore_secrets(&self, secrets: &[BackupSecret]) -> Result<SecretRestoreReport, AppError> {
        let mut report = SecretRestoreReport::default();
        for secret in secrets {
            let key = SecretKey::new(secret.owner, secret.kind);
            match self.secrets.get(&key) {
                // 本机已有同名口令：保留现有值，导入备份不改本机正在用的密码。
                Ok(Some(_)) => report.kept += 1,
                Ok(None) => match self.secrets.set(&key, secret.value.as_str()) {
                    Ok(()) => report.restored += 1,
                    Err(error) => {
                        report.failed += 1;
                        log::warn!(
                            "备份口令写入钥匙串失败，归属: {:?}，类型: {:?}，原因: {error}",
                            secret.owner,
                            secret.kind
                        );
                    }
                },
                Err(error) => {
                    return Err(AppError::with_detail(
                        ErrorCode::SecretStoreUnavailable,
                        format!("系统钥匙串不可用，备份口令未能写回：{error}"),
                    ));
                }
            }
        }
        Ok(report)
    }
}

/// 写入备份文件：给了口令就加密，没给就写明文 JSON/YAML。
///
/// 与存储文件共用同一套原子写：唯一临时名 + fsync + rename 覆盖，0600
/// 权限由私有句柄在打开临时文件时收紧；导出后立即断电也不会得到空文件。
///
/// # Errors
/// 序列化/加密、父目录创建、写盘或重命名失败时返回 `AppError`。
pub fn write_backup(
    path: &Path,
    document: &BackupDocument,
    passphrase: Option<&str>,
    format: BackupFormat,
) -> Result<(), AppError> {
    let data = match passphrase {
        Some(passphrase) => encrypt_document(document, passphrase, format)?,
        None => serialize_document(document, format)?,
    };
    crate::file_io::write_atomic(path, &data)
}

/// 读取备份文件：给了口令按加密文件解密，没给按明文文件解析。
///
/// # Errors
/// 文件不存在、超过大小上限、读取失败、口令错误或格式不符时返回 `AppError`。
pub fn read_backup(path: &Path, passphrase: Option<&str>) -> Result<BackupDocument, AppError> {
    let bytes = read_backup_bytes(path)?;
    match passphrase {
        Some(passphrase) => {
            if !is_encrypted_envelope(&bytes) {
                return Err(AppError::with_detail(
                    ErrorCode::BackupNotEncrypted,
                    "该文件不是加密备份，请取消「已加密」勾选后重试",
                ));
            }
            decrypt_document(&bytes, passphrase)
        }
        None => {
            if is_encrypted_envelope(&bytes) {
                return Err(AppError::with_detail(
                    ErrorCode::BackupEncrypted,
                    "该文件是加密备份，请勾选「已加密」并输入口令后重试",
                ));
            }
            let mut document = parse_document(&bytes)?;
            document.normalize();
            Ok(document)
        }
    }
}

/// 读取备份文件字节，并限制体积避免恶意超大文件。
fn read_backup_bytes(path: &Path) -> Result<Vec<u8>, AppError> {
    let metadata = fs::metadata(path).map_err(|error| {
        AppError::with_detail(
            ErrorCode::Io,
            format!("读取备份文件失败，路径: {}，原因: {error}", path.display()),
        )
    })?;
    if metadata.len() > MAX_BACKUP_BYTES {
        return Err(AppError::with_detail(
            ErrorCode::Configuration,
            "备份文件超过 16 MiB 上限，已拒绝导入",
        ));
    }
    fs::read(path).map_err(AppError::from)
}

/// 解析明文备份文档。
fn parse_document(bytes: &[u8]) -> Result<BackupDocument, AppError> {
    serde_json::from_slice(bytes)
        .or_else(|_| serde_yaml_ng::from_slice(bytes))
        .map_err(|_| {
            AppError::with_detail(
                ErrorCode::Configuration,
                "备份文件无法解析，可能不是 Transom 会话备份",
            )
        })
}

/// 序列化明文备份文档。
fn serialize_document(
    document: &BackupDocument,
    format: BackupFormat,
) -> Result<Vec<u8>, AppError> {
    match format {
        BackupFormat::Json => serde_json::to_vec_pretty(document).map_err(|error| {
            AppError::with_detail(ErrorCode::Configuration, format!("序列化备份失败: {error}"))
        }),
        BackupFormat::Yaml => serde_yaml_ng::to_string(document)
            .map(String::into_bytes)
            .map_err(|error| {
                AppError::with_detail(ErrorCode::Configuration, format!("序列化备份失败: {error}"))
            }),
    }
}

#[cfg(test)]
mod tests {
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
}
