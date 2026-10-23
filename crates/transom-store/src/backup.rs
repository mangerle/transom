//! 会话目录的口令加密备份。
//!
//! 安全设计：
//! - KDF 使用 Argon2id（64 MiB / 3 轮 / 单通道），固定参数写入代码而非
//!   文件，攻击者无法通过篡改文件把 KDF 降级为低成本参数。
//! - 加密使用 XChaCha20-Poly1305，24 字节随机 nonce 与 16 字节随机盐；
//!   AAD 绑定格式标记，避免密文被挪作其他用途。
//! - 明文只包含会话与文件夹配置，不读取系统钥匙串中的密码/口令。
//! - 文件以 0600 权限原子写入，导入前限制文件大小，防止恶意超大文件。

use argon2::{Algorithm, Argon2, Params, Version};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use transom_core::{AppError, BackupService, ErrorCode, SessionCatalog};
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

#[derive(Serialize, Deserialize)]
struct EncryptedBackup {
    format: String,
    version: u32,
    salt: String,
    nonce: String,
    ciphertext: String,
}

#[derive(Serialize, Deserialize)]
struct BackupPayload {
    version: u32,
    #[serde(default)]
    exported_at: u64,
    catalog: SessionCatalog,
}

/// 加密会话目录并序列化为 JSON 或 YAML。
///
/// # Errors
/// 口令为空、系统随机源不可用、序列化或加密失败时返回 `AppError`。
pub fn encrypt_catalog(
    catalog: &SessionCatalog,
    passphrase: &str,
    format: BackupFormat,
) -> Result<Vec<u8>, AppError> {
    if passphrase.is_empty() {
        return Err(AppError::with_detail(
            ErrorCode::Configuration,
            "备份口令不能为空",
        ));
    }
    let payload = BackupPayload {
        version: VERSION,
        exported_at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or(0),
        catalog: catalog.clone(),
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

/// 解密备份并返回会话目录快照。
///
/// # Errors
/// 文件格式、版本、口令或完整性校验失败时返回统一的中文错误，
/// 不区分“口令错误”和“密文损坏”，避免为暴力破解提供额外信息。
pub fn decrypt_catalog(bytes: &[u8], passphrase: &str) -> Result<SessionCatalog, AppError> {
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
    let payload: BackupPayload =
        serde_json::from_slice(&plaintext).map_err(|_| invalid_backup())?;
    if payload.version > VERSION {
        return Err(AppError::with_detail(
            ErrorCode::Configuration,
            "备份内容版本高于当前应用支持版本",
        ));
    }
    let mut catalog = payload.catalog;
    catalog.normalize();
    Ok(catalog)
}

/// 加密并原子写入备份文件，Unix 下权限固定为 0600。
///
/// # Errors
/// 目录创建、写盘或权限设置失败时返回 `AppError`。
pub fn write_encrypted_backup(
    path: &Path,
    catalog: &SessionCatalog,
    passphrase: &str,
    format: BackupFormat,
) -> Result<(), AppError> {
    let data = encrypt_catalog(catalog, passphrase, format)?;
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

/// 读取并解密备份文件。
///
/// # Errors
/// 文件不存在、超过大小上限、读取失败或解密失败时返回 `AppError`。
pub fn read_encrypted_backup(path: &Path, passphrase: &str) -> Result<SessionCatalog, AppError> {
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
    let bytes = fs::read(path).map_err(AppError::from)?;
    decrypt_catalog(&bytes, passphrase)
}

/// 基于文件扩展名选择 JSON/YAML 的加密备份实现。
pub struct EncryptedBackupService;

impl BackupService for EncryptedBackupService {
    fn export(
        &self,
        path: &Path,
        catalog: &SessionCatalog,
        passphrase: &str,
    ) -> Result<(), AppError> {
        write_encrypted_backup(path, catalog, passphrase, BackupFormat::from_path(path))
    }

    fn import(&self, path: &Path, passphrase: &str) -> Result<SessionCatalog, AppError> {
        read_encrypted_backup(path, passphrase)
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
    use transom_core::{AuthMethod, Connection, Session, SshConnection};

    fn catalog() -> SessionCatalog {
        let mut catalog = SessionCatalog::default();
        let folder = catalog.ensure_folder_path("生产/数据库").unwrap();
        let mut session = Session::new(
            "db-01",
            Connection::Ssh(SshConnection {
                host: "db.internal.example.com".into(),
                port: 22,
                username: "deploy".into(),
                auth: AuthMethod::Agent,
                options: Default::default(),
            }),
        );
        session.folder_id = Some(folder);
        catalog.sessions.push(session);
        catalog
    }

    #[test]
    fn json_and_yaml_round_trip() {
        for format in [BackupFormat::Json, BackupFormat::Yaml] {
            let encrypted = encrypt_catalog(&catalog(), "correct horse", format).unwrap();
            let text = String::from_utf8_lossy(&encrypted);
            assert!(!text.contains("db.internal.example.com"));
            let restored = decrypt_catalog(&encrypted, "correct horse").unwrap();
            assert_eq!(restored.sessions.len(), 1);
            assert_eq!(restored.folders.len(), 2);
            assert_eq!(
                restored.sessions[0].folder_id,
                restored.folders.last().map(|folder| folder.id)
            );
        }
    }

    #[test]
    fn wrong_passphrase_and_tampering_are_rejected() {
        let encrypted = encrypt_catalog(&catalog(), "secret", BackupFormat::Json).unwrap();
        assert!(decrypt_catalog(&encrypted, "wrong").is_err());

        let mut tampered = encrypted.clone();
        let last = tampered.len() - 2;
        tampered[last] = if tampered[last] == b'A' { b'B' } else { b'A' };
        assert!(decrypt_catalog(&tampered, "secret").is_err());
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
        assert!(decrypt_catalog(&bytes, "x").is_err());

        let envelope = EncryptedBackup {
            format: MAGIC.into(),
            version: VERSION + 1,
            ..envelope
        };
        let bytes = serde_json::to_vec(&envelope).unwrap();
        assert!(decrypt_catalog(&bytes, "x").is_err());
    }

    #[test]
    fn file_round_trip_and_format_detection() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("backup.yaml");
        assert_eq!(BackupFormat::from_path(&path), BackupFormat::Yaml);
        write_encrypted_backup(&path, &catalog(), "passphrase", BackupFormat::Yaml).unwrap();
        let loaded = read_encrypted_backup(&path, "passphrase").unwrap();
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
        assert!(encrypt_catalog(&catalog(), "", BackupFormat::Json).is_err());
    }
}
