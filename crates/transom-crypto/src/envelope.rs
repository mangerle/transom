//! 端到端加密同步信封：紧凑二进制头 + XChaCha20-Poly1305 密文。
//!
//! 设计原理（Why）：
//! - 同步目标可能是公开 S3 桶或私有 Git 仓库，对端只能看到不可解密的密文；
//!   把信封做成固定二进制布局（而非 JSON），体积小、字段不可被随意改写；
//! - 布局：`Magic(4) | Version(1) | Salt(16) | Nonce(24) | Ciphertext+Tag`；
//!   Poly1305 认证标签由 AEAD 附在密文尾部，任何一字节篡改都会解密失败；
//! - 密钥由 Argon2id 从用户口令派生，参数固定写死在代码里（64 MiB / 3 轮 /
//!   单通道），攻击者无法通过篡改文件把 KDF 降级为低成本参数；
//! - 明文与密钥在内存中用 `Zeroizing` 包裹，函数返回即清零。
//!
//! 代价与局限：每次同步都重新派生密钥（约 0.1~0.3 秒），换来的是不需要在
//! 磁盘上缓存派生密钥；大文件同步前先整体读入内存，上限由调用方控制。

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use transom_core::{AppError, ErrorCode};
use zeroize::Zeroizing;

/// 信封魔数：`TSYN`（Transom Sync）。
pub const ENVELOPE_MAGIC: [u8; 4] = *b"TSYN";
/// 当前信封版本。
pub const ENVELOPE_VERSION: u8 = 1;
/// 头部长度：magic + version + salt + nonce。
pub const ENVELOPE_HEADER_LEN: usize = 4 + 1 + 16 + 24;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 24;
/// Poly1305 标签长度（AEAD 附在密文尾部）。
const TAG_LEN: usize = 16;
const AAD: &[u8] = b"transom-sync-v1";

/// 是否是本模块生成的信封（只看魔数与版本，不做解密）。
pub fn is_envelope(bytes: &[u8]) -> bool {
    bytes.len() >= ENVELOPE_HEADER_LEN + TAG_LEN
        && bytes[..4] == ENVELOPE_MAGIC
        && bytes[4] == ENVELOPE_VERSION
}

/// 把明文封入加密信封。
///
/// # Errors
/// 安全随机数不可用、Argon2 参数非法或加密失败时返回 `CloudSyncFailed`。
pub fn seal(plaintext: &[u8], passphrase: &str) -> Result<Vec<u8>, AppError> {
    let salt = random_bytes::<SALT_LEN>()?;
    let key = crate::kdf::derive_key(passphrase, &salt)
        .map_err(|detail| AppError::with_detail(ErrorCode::CloudSyncFailed, detail))?;
    let nonce_bytes = random_bytes::<NONCE_LEN>()?;
    let cipher = XChaCha20Poly1305::new(key.as_ref().into());
    let ciphertext = cipher
        .encrypt(
            XNonce::from_slice(&nonce_bytes),
            Payload {
                msg: plaintext,
                aad: AAD,
            },
        )
        .map_err(|_| AppError::with_detail(ErrorCode::CloudSyncFailed, "同步信封加密失败"))?;
    let mut envelope = Vec::with_capacity(ENVELOPE_HEADER_LEN + ciphertext.len());
    envelope.extend_from_slice(&ENVELOPE_MAGIC);
    envelope.push(ENVELOPE_VERSION);
    envelope.extend_from_slice(&salt);
    envelope.extend_from_slice(&nonce_bytes);
    envelope.extend_from_slice(&ciphertext);
    Ok(envelope)
}

/// 打开加密信封，返回明文。
///
/// # Errors
/// 信封过短、魔数/版本不符、口令错误或密文被篡改时返回 `CloudSyncFailed`；
/// 错误信息不区分"口令错"与"密文坏"，避免给攻击者提供判定线索。
pub fn open(envelope: &[u8], passphrase: &str) -> Result<Zeroizing<Vec<u8>>, AppError> {
    if envelope.len() < ENVELOPE_HEADER_LEN + TAG_LEN {
        return Err(AppError::with_detail(
            ErrorCode::CloudSyncFailed,
            "同步信封长度不足，文件可能被截断",
        ));
    }
    if envelope[..4] != ENVELOPE_MAGIC {
        return Err(AppError::with_detail(
            ErrorCode::CloudSyncFailed,
            "同步信封魔数不匹配，文件不是 Transom 同步信封",
        ));
    }
    if envelope[4] != ENVELOPE_VERSION {
        return Err(AppError::with_detail(
            ErrorCode::CloudSyncFailed,
            format!("不支持的同步信封版本: {}", envelope[4]),
        ));
    }
    let salt = &envelope[5..5 + SALT_LEN];
    let nonce = &envelope[5 + SALT_LEN..ENVELOPE_HEADER_LEN];
    let ciphertext = &envelope[ENVELOPE_HEADER_LEN..];
    let key = crate::kdf::derive_key(passphrase, salt)
        .map_err(|detail| AppError::with_detail(ErrorCode::CloudSyncFailed, detail))?;
    let cipher = XChaCha20Poly1305::new(key.as_ref().into());
    let plaintext = cipher
        .decrypt(
            XNonce::from_slice(nonce),
            Payload {
                msg: ciphertext,
                aad: AAD,
            },
        )
        .map_err(|_| {
            AppError::with_detail(
                ErrorCode::CloudSyncFailed,
                "同步信封解密失败：口令错误或文件已被篡改",
            )
        })?;
    Ok(Zeroizing::new(plaintext))
}

fn random_bytes<const N: usize>() -> Result<[u8; N], AppError> {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes).map_err(|error| {
        AppError::with_detail(
            ErrorCode::CloudSyncFailed,
            format!("生成同步信封随机参数失败: {error}"),
        )
    })?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_rejects_wrong_passphrase() {
        let plaintext = br#"{"sessions":[{"name":"web-01"}]}"#;
        let envelope = seal(plaintext, "correct horse battery staple").unwrap();
        assert!(is_envelope(&envelope));
        assert_eq!(
            open(&envelope, "correct horse battery staple")
                .unwrap()
                .as_slice(),
            plaintext
        );
        assert!(open(&envelope, "wrong").is_err());
    }

    /// 每个信封的盐与 nonce 都不同：同一明文两次封包不应得到相同密文。
    #[test]
    fn envelopes_are_randomized() {
        let first = seal(b"same", "pass").unwrap();
        let second = seal(b"same", "pass").unwrap();
        assert_ne!(
            &first[5..ENVELOPE_HEADER_LEN],
            &second[5..ENVELOPE_HEADER_LEN]
        );
        assert_ne!(first, second);
    }

    #[test]
    fn rejects_tampering_truncation_and_foreign_files() {
        let mut envelope = seal(b"sensitive", "pass").unwrap();
        let last = envelope.len() - 1;
        envelope[last] ^= 0xFF;
        assert!(open(&envelope, "pass").is_err());

        let envelope = seal(b"sensitive", "pass").unwrap();
        assert!(open(&envelope[..envelope.len() - 1], "pass").is_err());
        assert!(!is_envelope(b"not an envelope"));
        assert!(open(b"short", "pass").is_err());
    }
}
