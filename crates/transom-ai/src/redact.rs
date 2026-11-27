//! 模型侧敏感信息自动脱敏。
//!
//! 设计原理（Why）：
//! - 远端命令输出、终端屏幕或配置文件读取结果中可能包含明文私钥、Bearer Token、URL 账密或 API Key；
//! - 在内容进入大模型上下文前统一遮蔽，即使模型请求了非预期文件也能避免明文凭据外泄给第三方接口。

use std::sync::LazyLock;

use regex::Regex;

static PEM_KEY_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z0-9 ]*PRIVATE KEY-----")
        .expect("PEM 私钥正则必须合法")
});

static BEARER_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(Bearer)\s+[A-Za-z0-9._~+/=-]{8,}").expect("Bearer 正则必须合法")
});

static URL_CRED_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)([a-z][a-z0-9+.-]*://[^\s:/@]+:)([^\s@]+)(@)").expect("URL 账密正则必须合法")
});

static KV_SECRET_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)\b([A-Za-z0-9_.-]*(?:password|passwd|passphrase|pwd|token|secret|api[_-]?key|access[_-]?key|secret[_-]?key|private[_-]?key)\b)\s*([:=])\s*(?:"[^"\n]+"|'[^'\n]+'|[^\s"',;]+)"#,
    )
    .expect("键值密钥正则必须合法")
});

static KNOWN_TOKEN_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(?:sk-[A-Za-z0-9_-]{16,}|gh[pousr]_[A-Za-z0-9]{20,}|AKIA[0-9A-Z]{16})\b")
        .expect("已知密钥前缀正则必须合法")
});

/// 在内容发往模型前自动遮蔽常见明文私钥、Token、URL 账密与配置密钥。
pub fn redact_secrets_for_model(raw: &str) -> String {
    if raw.is_empty() {
        return String::new();
    }
    let s1 = PEM_KEY_RE.replace_all(raw, "[REDACTED_PRIVATE_KEY]");
    let s2 = BEARER_RE.replace_all(&s1, "$1 [REDACTED]");
    let s3 = URL_CRED_RE.replace_all(&s2, "$1[REDACTED]$3");
    let s4 = KV_SECRET_RE.replace_all(&s3, "$1$2[REDACTED]");
    let s5 = KNOWN_TOKEN_RE.replace_all(&s4, "[REDACTED_TOKEN]");
    s5.into_owned()
}

#[cfg(test)]
mod tests {
    use super::redact_secrets_for_model;

    #[test]
    fn redacts_pem_bearer_url_kv_and_known_tokens() {
        let raw = "-----BEGIN RSA PRIVATE KEY-----\nMIIEowIBAAK\n-----END RSA PRIVATE KEY-----\n\
            Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.abcdefg\n\
            https://deploy:s3cr3tPass@git.example.com/repo.git\n\
            DB_PASSWORD=\"super_secret_123\"\n\
            api_key=sk-1234567890abcdef123456\n\
            ghp_123456789012345678901234567890\n\
            2026-11-26 INFO nginx started on port 80";
        let out = redact_secrets_for_model(raw);
        assert!(out.contains("[REDACTED_PRIVATE_KEY]"));
        assert!(!out.contains("MIIEowIBAAK"));
        assert!(out.contains("Bearer [REDACTED]"));
        assert!(!out.contains("eyJhbGciOiJIUzI1NiJ9"));
        assert!(out.contains("https://deploy:[REDACTED]@git.example.com/repo.git"));
        assert!(!out.contains("s3cr3tPass"));
        assert!(out.contains("DB_PASSWORD=[REDACTED]"));
        assert!(!out.contains("super_secret_123"));
        assert!(!out.contains("ghp_1234567890"));
        assert!(out.contains("INFO nginx started on port 80"));
    }
}
