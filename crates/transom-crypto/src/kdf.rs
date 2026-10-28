//! Argon2id 密钥派生：TSYN 二进制信封与 JSON/YAML 内嵌信封共用。
//!
//! 设计原理（Why）：两套信封此前各写一份派生函数，参数虽然当前一致，但
//! 强度调整只要漏改一处，两类密文的安全边界就会分叉。收敛到本模块后只有
//! 一组参数常量；盐长度与输出长度由调用方在解析边界校验（两处都固定 16
//! 字节盐），本模块只负责派生与参数校验。

use argon2::{Algorithm, Argon2, Params, Version};
use zeroize::Zeroizing;

/// 派生密钥长度。
const KEY_LEN: usize = 32;
/// 内存成本（KiB）。
const MEMORY_KIB: u32 = 64 * 1024;
/// 迭代次数。
const ITERATIONS: u32 = 3;
/// 并行度。
const PARALLELISM: u32 = 1;

/// 由口令与盐派生 32 字节密钥。
///
/// # Errors
/// 参数非法（只可能来自常量写错）或盐/输出长度不满足 Argon2 约束时，
/// 返回可直接拼进上层错误信息的中文原因。
pub(crate) fn derive_key(
    passphrase: &str,
    salt: &[u8],
) -> Result<Zeroizing<[u8; KEY_LEN]>, String> {
    let params = Params::new(MEMORY_KIB, ITERATIONS, PARALLELISM, Some(KEY_LEN))
        .map_err(|error| format!("Argon2 参数非法: {error}"))?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = Zeroizing::new([0u8; KEY_LEN]);
    argon
        .hash_password_into(passphrase.as_bytes(), salt, key.as_mut())
        .map_err(|error| format!("Argon2 密钥派生失败: {error}"))?;
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::derive_key;

    const SALT: &[u8] = b"0123456789abcdef";

    #[test]
    fn derivation_is_deterministic_for_same_inputs() {
        let first = derive_key("口令", SALT).expect("派生应成功");
        let second = derive_key("口令", SALT).expect("派生应成功");
        assert_eq!(
            first.as_ref(),
            second.as_ref(),
            "同盐同口令必须得到同一密钥"
        );
    }

    #[test]
    fn different_salt_produces_different_key() {
        let first = derive_key("口令", SALT).expect("派生应成功");
        let second = derive_key("口令", b"fedcba9876543210").expect("派生应成功");
        assert_ne!(first.as_ref(), second.as_ref(), "不同盐必须得到不同密钥");
    }

    #[test]
    fn too_short_salt_is_rejected() {
        assert!(
            derive_key("口令", b"short").is_err(),
            "少于 8 字节的盐必须拒绝，不能静默派生弱密钥"
        );
    }
}
