//! WebDAV HTTP Digest 认证解析与摘要计算。

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use md5::{Digest as _, Md5};
use reqwest::header::WWW_AUTHENTICATE;
use sha2::Sha256;
use transom_core::{AppError, ErrorCode};

static CNONCE_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Digest 挑战与当前 nonce 的请求计数。
pub(super) struct DigestState {
    pub(super) challenge: DigestChallenge,
    pub(super) nonce_count: u32,
}

/// 解析后的 Digest 必需字段。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DigestChallenge {
    pub(super) realm: String,
    pub(super) nonce: String,
    pub(super) algorithm: DigestAlgorithm,
    pub(super) qop: Option<String>,
    pub(super) opaque: Option<String>,
}

/// 支持的摘要算法。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DigestAlgorithm {
    Md5,
    Md5Sess,
    Sha256,
    Sha256Sess,
}

impl DigestChallenge {
    pub(super) fn authorization(
        &self,
        method: &str,
        uri: &str,
        username: &str,
        password: &str,
        nonce_count: u32,
        cnonce: &str,
    ) -> Result<String, AppError> {
        let nc = format!("{nonce_count:08x}");
        let ha1 = digest_hex(
            self.algorithm,
            format!("{username}:{}:{password}", self.realm),
        );
        let ha1 = if matches!(
            self.algorithm,
            DigestAlgorithm::Md5Sess | DigestAlgorithm::Sha256Sess
        ) {
            digest_hex(self.algorithm, format!("{ha1}:{}:{cnonce}", self.nonce))
        } else {
            ha1
        };
        let ha2 = digest_hex(self.algorithm, format!("{method}:{uri}"));
        let response = match self.qop.as_deref() {
            Some("auth") => digest_hex(
                self.algorithm,
                format!("{ha1}:{}:{nc}:{cnonce}:auth:{ha2}", self.nonce),
            ),
            Some(_) => {
                return Err(AppError::with_detail(
                    ErrorCode::CloudSyncFailed,
                    "WebDAV Digest 仅支持 qop=auth，服务器要求了不受支持的认证强度",
                ));
            }
            None => digest_hex(self.algorithm, format!("{ha1}:{}:{ha2}", self.nonce)),
        };
        let mut fields = vec![
            format!("username=\"{}\"", quote_escape(username)),
            format!("realm=\"{}\"", quote_escape(&self.realm)),
            format!("nonce=\"{}\"", quote_escape(&self.nonce)),
            format!("uri=\"{}\"", quote_escape(uri)),
            format!("response=\"{response}\""),
            format!("algorithm={}", self.algorithm.name()),
        ];
        if let Some(qop) = &self.qop {
            fields.push(format!("qop={qop}"));
            fields.push(format!("nc={nc}"));
            fields.push(format!("cnonce=\"{}\"", quote_escape(cnonce)));
        }
        if let Some(opaque) = &self.opaque {
            fields.push(format!("opaque=\"{}\"", quote_escape(opaque)));
        }
        Ok(format!("Digest {}", fields.join(", ")))
    }
}

impl DigestAlgorithm {
    fn name(self) -> &'static str {
        match self {
            Self::Md5 => "MD5",
            Self::Md5Sess => "MD5-sess",
            Self::Sha256 => "SHA-256",
            Self::Sha256Sess => "SHA-256-sess",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_uppercase().as_str() {
            "MD5" | "" => Some(Self::Md5),
            "MD5-SESS" => Some(Self::Md5Sess),
            "SHA-256" => Some(Self::Sha256),
            "SHA-256-SESS" => Some(Self::Sha256Sess),
            _ => None,
        }
    }
}

fn digest_hex(algorithm: DigestAlgorithm, value: impl AsRef<[u8]>) -> String {
    match algorithm {
        DigestAlgorithm::Md5 | DigestAlgorithm::Md5Sess => hex::encode(Md5::digest(value.as_ref())),
        DigestAlgorithm::Sha256 | DigestAlgorithm::Sha256Sess => {
            hex::encode(Sha256::digest(value.as_ref()))
        }
    }
}

pub(super) fn digest_challenge(response: &reqwest::Response) -> Option<DigestChallenge> {
    response
        .headers()
        .get_all(WWW_AUTHENTICATE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find_map(parse_digest_challenge)
}

pub(super) fn parse_digest_challenge(header: &str) -> Option<DigestChallenge> {
    let lower = header.to_ascii_lowercase();
    let digest_start = lower.find("digest ")?;
    let fields = parse_auth_fields(&header[digest_start + "digest ".len()..]);
    let realm = fields.get("realm")?.clone();
    let nonce = fields.get("nonce")?.clone();
    let algorithm = DigestAlgorithm::parse(fields.get("algorithm").map_or("", String::as_str))?;
    let qop = fields.get("qop").and_then(|value| {
        value
            .split(',')
            .map(str::trim)
            .find(|candidate| *candidate == "auth")
            .map(|_| "auth".to_string())
    });
    if fields.contains_key("qop") && qop.is_none() {
        return None;
    }
    Some(DigestChallenge {
        realm,
        nonce,
        algorithm,
        qop,
        opaque: fields.get("opaque").cloned(),
    })
}

fn parse_auth_fields(input: &str) -> BTreeMap<String, String> {
    let mut fields = BTreeMap::new();
    let bytes = input.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        while index < bytes.len() && matches!(bytes[index], b' ' | b',' | b'\t') {
            index += 1;
        }
        let key_start = index;
        while index < bytes.len() && bytes[index] != b'=' && bytes[index] != b',' {
            index += 1;
        }
        if index >= bytes.len() || bytes[index] != b'=' {
            break;
        }
        let key = input[key_start..index].trim().to_ascii_lowercase();
        index += 1;
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        let value = if index < bytes.len() && bytes[index] == b'"' {
            index += 1;
            let start = index;
            let mut escaped = false;
            while index < bytes.len() {
                match bytes[index] {
                    b'"' if !escaped => break,
                    b'\\' if !escaped => escaped = true,
                    _ => escaped = false,
                }
                index += 1;
            }
            let value = input[start..index].replace("\\\"", "\"");
            index = (index + 1).min(bytes.len());
            value
        } else {
            let start = index;
            while index < bytes.len() && bytes[index] != b',' {
                index += 1;
            }
            input[start..index].trim().to_string()
        };
        if !key.is_empty() {
            fields.insert(key, value);
        }
    }
    fields
}

pub(super) fn digest_uri(url: &reqwest::Url) -> String {
    let mut uri = url.path().to_string();
    if let Some(query) = url.query() {
        uri.push('?');
        uri.push_str(query);
    }
    uri
}

fn quote_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

pub(super) fn next_cnonce() -> String {
    let counter = CNONCE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("{nanos:x}{counter:08x}")
}
