//! 会话目录的备份文件读写：明文 JSON/YAML 或口令加密信封。
//!
//! 安全设计：
//! - KDF 使用 Argon2id（64 MiB / 3 轮 / 单通道），固定参数写入代码而非
//!   文件，攻击者无法通过篡改文件把 KDF 降级为低成本参数。
//! - 加密使用 XChaCha20-Poly1305，24 字节随机 nonce 与 16 字节随机盐；
//!   AAD 绑定格式标记，避免密文被挪作其他用途。
//! - 明文备份默认不含系统钥匙串里的密码/口令，只有用户显式勾选才会写入。
//! - 文件以 0600 权限原子写入，导入前限制文件大小，防止恶意超大文件。

use argon2::{Algorithm, Argon2, Params, Version};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use transom_core::{
    AppError, BackupDocument, BackupExportOptions, BackupExportReport, BackupImportOptions,
    BackupSecret, BackupService, ErrorCode, SecretKey, SecretRestoreReport, SecretStore,
    SessionCatalog,
};
use zeroize::Zeroizing;

const MAGIC: &str = "transom-backup";
const VERSION: u32 = 1;
const AAD: &[u8] = b"transom-backup-v1";
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 24;
/// 备份仅含配置文本，16 MiB 足以容纳数万条会话，同时限制恶意文件。
const MAX_BACKUP_BYTES: u64 = 16 * 1024 * 1024;
const ARGON_MEMORY_KIB: u32 = 64 * 1024;
const ARGON_ITERATIONS: u32 = 3;
const ARGON_PARALLELISM: u32 = 1;

/// 备份文件的序列化格式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackupFormat {
    Json,
    Yaml,
}

impl BackupFormat {
    /// 根据扩展名选择格式，未知扩展名默认 JSON。
    pub fn from_path(path: &Path) -> Self {
        match path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("yaml" | "yml") => Self::Yaml,
            _ => Self::Json,
        }
    }
}

/// 加密信封：文件里只有随机参数与密文，看不出任何会话信息。
#[derive(Serialize, Deserialize)]
struct EncryptedBackup {
    format: String,
    version: u32,
    salt: String,
    nonce: String,
    ciphertext: String,
}

/// 信封内的明文负载，按借用写出，避免为了加密再克隆一份口令。
#[derive(Serialize)]
struct EncryptedPayloadRef<'a> {
    version: u32,
    document: &'a BackupDocument,
}

/// 信封内的明文负载结构。
#[derive(Deserialize)]
struct EncryptedPayload {
    version: u32,
    document: BackupDocument,
}

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

    /// 按会话的认证方式收集钥匙串里保存的口令。
    ///
    /// 钥匙串整体不可用时直接失败：此时导出"含口令"的备份只会得到一份
    /// 缺口的文件，用户很难察觉，必须让他显式取消勾选或修复钥匙串。
    fn collect_secrets(&self, catalog: &SessionCatalog) -> Result<Vec<BackupSecret>, AppError> {
        let mut secrets = Vec::new();
        for session in &catalog.sessions {
            for kind in session.secret_kinds() {
                let key = SecretKey::new(session.id, *kind);
                match self.secrets.get(&key) {
                    Ok(Some(value)) => secrets.push(BackupSecret {
                        session_id: session.id,
                        kind: *kind,
                        value,
                    }),
                    Ok(None) => {}
                    Err(_error) => {
                        return Err(AppError::with_detail(
                            ErrorCode::SecretStoreUnavailable,
                            format!(
                                "读取会话「{}」的口令失败：系统钥匙串不可用；请取消「包含已保存的口令」后重试",
                                session.name
                            ),
                        ));
                    }
                }
            }
        }
        Ok(secrets)
    }
}

impl BackupService for CatalogBackupService {
    fn export(
        &self,
        path: &Path,
        catalog: &SessionCatalog,
        options: &BackupExportOptions,
    ) -> Result<BackupExportReport, AppError> {
        let mut document = BackupDocument::from_catalog(catalog);
        if options.include_secrets {
            document.secrets = Some(self.collect_secrets(catalog)?);
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
            let key = SecretKey::new(secret.session_id, secret.kind);
            match self.secrets.get(&key) {
                // 本机已有同名口令：保留现有值，导入备份不改本机正在用的密码。
                Ok(Some(_)) => report.kept += 1,
                Ok(None) => match self.secrets.set(&key, secret.value.as_str()) {
                    Ok(()) => report.restored += 1,
                    Err(error) => {
                        report.failed += 1;
                        tracing::warn!(
                            session_id = %secret.session_id,
                            kind = ?secret.kind,
                            %error,
                            "备份口令写入钥匙串失败"
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

/// 加密备份文档并序列化为 JSON 或 YAML。
///
/// # Errors
/// 口令为空、系统随机源不可用、序列化或加密失败时返回 `AppError`。
pub fn encrypt_document(
    document: &BackupDocument,
    passphrase: &str,
    format: BackupFormat,
) -> Result<Vec<u8>, AppError> {
    if passphrase.is_empty() {
        return Err(AppError::with_detail(
            ErrorCode::Configuration,
            "备份口令不能为空",
        ));
    }
    let payload = EncryptedPayloadRef {
        version: VERSION,
        document,
    };
    let plaintext = Zeroizing::new(serde_json::to_vec(&payload).map_err(|error| {
        AppError::with_detail(
            ErrorCode::Configuration,
            format!("序列化备份内容失败: {error}"),
        )
    })?);

    let mut salt = [0u8; SALT_LEN];
    getrandom::fill(&mut salt).map_err(|error| {
        AppError::with_detail(ErrorCode::Io, format!("生成备份随机盐失败: {error}"))
    })?;
    let mut nonce = [0u8; NONCE_LEN];
    getrandom::fill(&mut nonce).map_err(|error| {
        AppError::with_detail(ErrorCode::Io, format!("生成备份随机数失败: {error}"))
    })?;

    let key = derive_key(passphrase, &salt)?;
    let cipher = XChaCha20Poly1305::new_from_slice(&key[..])
        .map_err(|_| AppError::with_detail(ErrorCode::Configuration, "初始化备份加密器失败"))?;
    let ciphertext = cipher
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: &plaintext,
                aad: AAD,
            },
        )
        .map_err(|_| AppError::with_detail(ErrorCode::Configuration, "加密备份内容失败"))?;

    let envelope = EncryptedBackup {
        format: MAGIC.to_string(),
        version: VERSION,
        salt: BASE64.encode(salt),
        nonce: BASE64.encode(nonce),
        ciphertext: BASE64.encode(ciphertext),
    };
    serialize_envelope(&envelope, format)
}

/// 解密备份并返回文档快照。
///
/// # Errors
/// 文件格式、版本、口令或完整性校验失败时返回统一的中文错误，
/// 不区分“口令错误”和“密文损坏”，避免为暴力破解提供额外信息。
pub fn decrypt_document(bytes: &[u8], passphrase: &str) -> Result<BackupDocument, AppError> {
    let envelope = parse_envelope(bytes)?;
    if envelope.format != MAGIC || envelope.version > VERSION {
        return Err(AppError::with_detail(
            ErrorCode::Configuration,
            "不是受支持的 Transom 备份文件或版本过新",
        ));
    }
    let salt = BASE64
        .decode(&envelope.salt)
        .map_err(|_| invalid_backup())?;
    let nonce = BASE64
        .decode(&envelope.nonce)
        .map_err(|_| invalid_backup())?;
    let ciphertext = BASE64
        .decode(&envelope.ciphertext)
        .map_err(|_| invalid_backup())?;
    if salt.len() != SALT_LEN || nonce.len() != NONCE_LEN {
        return Err(invalid_backup());
    }

    let key = derive_key(passphrase, &salt)?;
    let cipher = XChaCha20Poly1305::new_from_slice(&key[..]).map_err(|_| invalid_backup())?;
    let plaintext = Zeroizing::new(
        cipher
            .decrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: &ciphertext,
                    aad: AAD,
                },
            )
            .map_err(|_| invalid_backup())?,
    );
    let payload: EncryptedPayload =
        serde_json::from_slice(&plaintext).map_err(|_| invalid_backup())?;
    if payload.version > VERSION {
        return Err(AppError::with_detail(
            ErrorCode::Configuration,
            "备份内容版本高于当前应用支持版本",
        ));
    }
    let mut document = payload.document;
    document.normalize();
    Ok(document)
}

/// 写入备份文件：给了口令就加密，没给就写明文 JSON/YAML。
///
/// 文件以 0600 权限原子落盘，Unix 下临时文件同样受限。
///
/// # Errors
/// 目录创建、序列化、写盘或权限设置失败时返回 `AppError`。
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
    let parent = path
        .parent()
        .ok_or_else(|| AppError::with_detail(ErrorCode::Configuration, "备份文件路径无效"))?;
    fs::create_dir_all(parent).map_err(AppError::from)?;
    let temporary = temporary_path(path);
    fs::write(&temporary, data).map_err(AppError::from)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))
            .map_err(AppError::from)?;
    }
    #[cfg(windows)]
    {
        // Windows 的 rename 不会覆盖已存在文件；文件选择器已确认覆盖意图。
        let _ = fs::remove_file(path);
    }
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(AppError::from(error));
    }
    Ok(())
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

/// 判断文件是否是加密信封。
///
/// 明文文档没有 `format` 标记，按信封解析必然失败；因此这里只需确认
/// "能按信封解析且标记匹配"，不必给明文格式另立一套字段。
fn is_encrypted_envelope(bytes: &[u8]) -> bool {
    parse_envelope(bytes).is_ok_and(|envelope| envelope.format == MAGIC)
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

fn derive_key(passphrase: &str, salt: &[u8]) -> Result<Zeroizing<[u8; 32]>, AppError> {
    let params = Params::new(
        ARGON_MEMORY_KIB,
        ARGON_ITERATIONS,
        ARGON_PARALLELISM,
        Some(32),
    )
    .map_err(|error| {
        AppError::with_detail(
            ErrorCode::Configuration,
            format!("初始化备份密钥派生参数失败: {error}"),
        )
    })?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = Zeroizing::new([0u8; 32]);
    argon
        .hash_password_into(passphrase.as_bytes(), salt, &mut key[..])
        .map_err(|error| {
            AppError::with_detail(
                ErrorCode::Configuration,
                format!("派生备份密钥失败: {error}"),
            )
        })?;
    Ok(key)
}

fn serialize_envelope(
    envelope: &EncryptedBackup,
    format: BackupFormat,
) -> Result<Vec<u8>, AppError> {
    match format {
        BackupFormat::Json => serde_json::to_vec_pretty(envelope).map_err(|error| {
            AppError::with_detail(
                ErrorCode::Configuration,
                format!("序列化加密备份失败: {error}"),
            )
        }),
        BackupFormat::Yaml => serde_yaml_ng::to_string(envelope)
            .map(String::into_bytes)
            .map_err(|error| {
                AppError::with_detail(
                    ErrorCode::Configuration,
                    format!("序列化加密备份失败: {error}"),
                )
            }),
    }
}

fn parse_envelope(bytes: &[u8]) -> Result<EncryptedBackup, AppError> {
    serde_json::from_slice(bytes)
        .or_else(|_| serde_yaml_ng::from_slice(bytes))
        .map_err(|_| invalid_backup())
}

fn invalid_backup() -> AppError {
    AppError::with_detail(ErrorCode::Configuration, "备份口令错误或文件已损坏")
}

fn temporary_path(path: &Path) -> PathBuf {
    let mut temporary = path.to_path_buf();
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| format!("{extension}.tmp"))
        .unwrap_or_else(|| "tmp".to_string());
    temporary.set_extension(extension);
    temporary
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use transom_core::{AuthMethod, Connection, SecretKind, Session, SshConnection};

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
    fn unknown_format_or_version_is_rejected() {
        let envelope = EncryptedBackup {
            format: "other".into(),
            version: 1,
            salt: BASE64.encode([0u8; SALT_LEN]),
            nonce: BASE64.encode([0u8; NONCE_LEN]),
            ciphertext: BASE64.encode([0u8; 8]),
        };
        let bytes = serde_json::to_vec(&envelope).unwrap();
        assert!(decrypt_document(&bytes, "x").is_err());

        let envelope = EncryptedBackup {
            format: MAGIC.into(),
            version: VERSION + 1,
            ..envelope
        };
        let bytes = serde_json::to_vec(&envelope).unwrap();
        assert!(decrypt_document(&bytes, "x").is_err());
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
    fn export_includes_secrets_only_when_requested() {
        let store = Arc::new(FakeSecretStore::available());
        let catalog = catalog();
        let key = SecretKey::new(catalog.sessions[0].id, SecretKind::Password);
        store.put(&key, "s3cret");
        let service = service(store.clone());

        let dir = tempdir().unwrap();
        let path = dir.path().join("with-secrets.json");
        let options = BackupExportOptions {
            include_secrets: true,
            ..Default::default()
        };
        let report = service
            .export(&path, &catalog, &options)
            .expect("导出应当成功");
        assert_eq!(report.secrets, 1);

        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("s3cret"));

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
            ..Default::default()
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
            session_id: catalog.sessions[0].id,
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
            session_id: catalog.sessions[0].id,
            kind: SecretKind::Password,
            value: Zeroizing::new("from-backup".to_string()),
        }];
        let report = service.restore_secrets(&secrets).unwrap();
        assert_eq!(report.restored, 1);
        let stored = store.get(&key).unwrap().unwrap();
        assert_eq!(stored.as_str(), "from-backup");
    }
}
