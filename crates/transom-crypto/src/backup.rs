//! 备份文档的加密信封：JSON/YAML 外层内嵌 base64 的 XChaCha20-Poly1305 密文。
//!
//! 与 [`super::envelope`] 的 TSYN 二进制信封是两种独立格式：本格式面向
//! "用户可见的备份文件"——明文时是干净的 JSON/YAML 文档，加密时在同样的
//! 外壳里填入格式标记、盐、nonce 与密文字段；TSYN 面向"对象存储里的不透明
//! 密文对象"。两者共用同一套 KDF 口径（Argon2id，64 MiB / 3 轮 / 单通道）
//! 与 XChaCha20-Poly1305，但魔数与 AAD 各自独立，密文不可跨格式挪用。
//!
//! 安全设计：
//! - KDF 参数固定写死，攻击者无法通过篡改文件把 KDF 降级为低成本参数；
//! - 错误信息不区分"口令错误"与"密文损坏"，避免为暴力破解提供判定线索；
//! - 明文负载以借用结构序列化，避免为加密再克隆一份口令。

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use serde::{Deserialize, Serialize};
use std::path::Path;
use transom_core::{AppError, BackupDocument, ErrorCode};
use zeroize::Zeroizing;

const MAGIC: &str = "transom-backup";
const VERSION: u32 = 1;
const AAD: &[u8] = b"transom-backup-v1";
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 24;
/// 备份仅含配置文本，16 MiB 足以容纳数万条会话，同时限制恶意文件体积。
pub const MAX_BACKUP_BYTES: u64 = 16 * 1024 * 1024;

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

    let key = crate::kdf::derive_key(passphrase, &salt)
        .map_err(|detail| AppError::with_detail(ErrorCode::Configuration, detail))?;
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

    let key = crate::kdf::derive_key(passphrase, &salt)
        .map_err(|detail| AppError::with_detail(ErrorCode::Configuration, detail))?;
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

/// 判断字节内容是否是加密信封。
///
/// 明文文档没有 `format` 标记，按信封解析必然失败；因此这里只需确认
/// "能按信封解析且标记匹配"，不必给明文格式另立一套字段。
pub fn is_encrypted_envelope(bytes: &[u8]) -> bool {
    parse_envelope(bytes).is_ok_and(|envelope| envelope.format == MAGIC)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 篡改格式标记或抬高版本号都必须被拒绝，且不得落入"按明文解析"的路径。
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
}
