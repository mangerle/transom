//! WebDAV 云同步端点：Basic/Digest 认证与最小 PUT/GET/HEAD/MKCOL 客户端。
//!
//! 设计说明（Why）：
//! - 同步协议只依赖固定对象，不枚举目录；PUT/GET/HEAD 与按需 MKCOL 已覆盖
//!   坚果云、Nextcloud 和常见 NAS 的必要能力；
//! - 先发送 Basic，收到 Digest 挑战后缓存并在后续请求预认证；这样兼容两类
//!   服务，又避免每次 PUT 都先上传一次必然失败的正文；
//! - Digest 只支持 `auth` qop 与 MD5/SHA-256 系列，不支持需要正文摘要的
//!   `auth-int`；遇到该能力时返回明确错误，而不是发送无法验证的请求。

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use md5::{Digest as _, Md5};
use parking_lot::Mutex;
use reqwest::header::{AUTHORIZATION, WWW_AUTHENTICATE};
use sha2::Sha256;
use transom_core::{AppError, CloudSyncTarget, ErrorCode, SYNC_OBJECT_NAME};
use zeroize::Zeroizing;

use super::download_limited;

static CNONCE_COUNTER: AtomicU64 = AtomicU64::new(1);

/// WebDAV 远端目标：固定对象地址、认证信息与按需目录创建能力。
pub struct WebDavTarget {
    http: reqwest::Client,
    object_url: reqwest::Url,
    /// 需要按序创建的目录地址（不含端点根目录本身）。
    directory_urls: Vec<reqwest::Url>,
    username: String,
    password: Option<Zeroizing<String>>,
    /// 首次 Basic 请求触发 401 后缓存的 Digest 挑战。
    digest: Mutex<Option<DigestState>>,
    /// 供错误信息定位的服务器主机名（不含凭据）。
    host: String,
}

/// Digest 挑战与当前 nonce 的请求计数。
struct DigestState {
    challenge: DigestChallenge,
    nonce_count: u32,
}

/// 解析后的 Digest 必需字段。
#[derive(Debug, Clone, PartialEq, Eq)]
struct DigestChallenge {
    realm: String,
    nonce: String,
    algorithm: DigestAlgorithm,
    qop: Option<String>,
    opaque: Option<String>,
}

/// 支持的摘要算法。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DigestAlgorithm {
    Md5,
    Md5Sess,
    Sha256,
    Sha256Sess,
}

impl WebDavTarget {
    /// 构造 WebDAV 目标。
    ///
    /// # Errors
    /// 根地址不是合法的 https URL，或目录段无法拼接时返回 `AppError`。
    pub fn new(
        http: reqwest::Client,
        base_url: &str,
        username: &str,
        password: Option<&str>,
        directory: &str,
    ) -> Result<Self, AppError> {
        let mut base = reqwest::Url::parse(base_url).map_err(|error| {
            AppError::with_detail(
                ErrorCode::Configuration,
                format!("WebDAV 地址无法解析: {error}"),
            )
        })?;
        // 丢弃 base 上可能携带的查询与片段，仅保留 scheme://host/path 前缀。
        base.set_query(None);
        base.set_fragment(None);
        if base.scheme() != "https" {
            return Err(AppError::with_detail(
                ErrorCode::Configuration,
                format!(
                    "WebDAV 地址必须使用 https（http 会明文传输同步口令与备份内容），当前为「{base_url}」"
                ),
            ));
        }
        let host = base
            .host_str()
            .unwrap_or("unknown")
            .trim_end_matches('/')
            .to_string();
        // `Url::join` 把无尾斜杠的最后一段当作文件；WebDAV 根地址通常没有
        // 尾斜杠，先显式补齐，避免把用户配置的目录前缀静默截掉。
        let mut base_path = base.path().trim_end_matches('/').to_string();
        base_path.push('/');
        base.set_path(&base_path);

        let mut directory_urls = Vec::new();
        let mut current = base;
        for segment in directory.split('/').filter(|segment| !segment.is_empty()) {
            current = current.join(&format!("{segment}/")).map_err(|error| {
                AppError::with_detail(
                    ErrorCode::Configuration,
                    format!("同步目录拼接失败: {error}"),
                )
            })?;
            directory_urls.push(current.clone());
        }
        let object_url = current.join(SYNC_OBJECT_NAME).map_err(|error| {
            AppError::with_detail(
                ErrorCode::Configuration,
                format!("同步对象地址拼接失败: {error}"),
            )
        })?;

        Ok(Self {
            http,
            object_url,
            directory_urls,
            username: username.to_string(),
            password: password.map(|value| Zeroizing::new(value.to_string())),
            digest: Mutex::new(None),
            host,
        })
    }

    /// 发送请求；遇到 Digest 挑战时更新认证信息并重试一次。
    async fn send(
        &self,
        method: reqwest::Method,
        url: &reqwest::Url,
        body: Option<&[u8]>,
        if_match: Option<&str>,
    ) -> Result<reqwest::Response, AppError> {
        let response = self
            .send_once(method.clone(), url, body, if_match, false)
            .await?;
        if response.status() != reqwest::StatusCode::UNAUTHORIZED {
            return Ok(response);
        }
        let Some(challenge) = digest_challenge(&response) else {
            return Ok(response);
        };
        if self.password.is_none() {
            return Ok(response);
        }
        let state = DigestState {
            challenge,
            nonce_count: 0,
        };
        *self.digest.lock() = Some(state);
        self.send_once(method, url, body, if_match, true).await
    }

    async fn send_once(
        &self,
        method: reqwest::Method,
        url: &reqwest::Url,
        body: Option<&[u8]>,
        if_match: Option<&str>,
        force_digest: bool,
    ) -> Result<reqwest::Response, AppError> {
        let authorization = self.authorization(&method, url, force_digest)?;
        let mut request = self.http.request(method, url.clone());
        if let Some(authorization) = authorization {
            request = request.header(AUTHORIZATION, authorization);
        }
        if let Some(version) = if_match {
            request = request.header(reqwest::header::IF_MATCH, version);
        }
        if let Some(body) = body {
            request = request.body(body.to_vec());
        }
        request
            .send()
            .await
            .map_err(|error| http_error("访问 WebDAV", &self.host, error))
    }

    fn authorization(
        &self,
        method: &reqwest::Method,
        url: &reqwest::Url,
        force_digest: bool,
    ) -> Result<Option<String>, AppError> {
        let Some(password) = self.password.as_deref() else {
            return Ok(None);
        };
        let mut state = self.digest.lock();
        if let Some(state) = state.as_mut() {
            state.nonce_count = state.nonce_count.saturating_add(1);
            let cnonce = next_cnonce();
            let header = state.challenge.authorization(
                method.as_str(),
                &digest_uri(url),
                &self.username,
                password,
                state.nonce_count,
                &cnonce,
            )?;
            return Ok(Some(header));
        }
        drop(state);
        if force_digest {
            return Err(AppError::with_detail(
                ErrorCode::CloudSyncFailed,
                format!(
                    "WebDAV 要求 Digest 认证，但服务器未返回可解析的挑战，服务器: {}",
                    self.host
                ),
            ));
        }
        let token = BASE64.encode(format!("{}:{}", self.username, password.as_str()));
        Ok(Some(format!("Basic {token}")))
    }

    /// 逐级创建目录；已存在的目录（405）视为成功。
    ///
    /// MKCOL 一次只建一级，逐级推进时同级失败立即中断，错误信息带上
    /// 主机名便于定位，绝不回显凭据。
    async fn ensure_directories(&self) -> Result<(), AppError> {
        for url in &self.directory_urls {
            let response = self
                .send(
                    reqwest::Method::from_bytes(b"MKCOL").expect("MKCOL 是合法方法名"),
                    url,
                    None,
                    None,
                )
                .await?;
            let status = response.status();
            if !status.is_success() && status != reqwest::StatusCode::METHOD_NOT_ALLOWED {
                return Err(AppError::with_detail(
                    ErrorCode::CloudSyncFailed,
                    format!("创建 WebDAV 目录失败，HTTP {status}，服务器: {url}"),
                ));
            }
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl CloudSyncTarget for WebDavTarget {
    async fn put_document(&self, data: &[u8]) -> Result<(), AppError> {
        self.put_document_if_match(data, None).await
    }

    async fn put_document_if_match(
        &self,
        data: &[u8],
        version: Option<&str>,
    ) -> Result<(), AppError> {
        let mut response = self
            .send(reqwest::Method::PUT, &self.object_url, Some(data), version)
            .await?;
        // 目录缺失在部分实现上表现为 404（路径不存在），部分为 409（父级缺失）。
        if response.status() == reqwest::StatusCode::NOT_FOUND
            || response.status() == reqwest::StatusCode::CONFLICT
        {
            self.ensure_directories().await?;
            response = self
                .send(reqwest::Method::PUT, &self.object_url, Some(data), version)
                .await?;
        }
        let status = response.status();
        if status == reqwest::StatusCode::PRECONDITION_FAILED {
            return Err(AppError::with_detail(
                ErrorCode::CloudSyncConflict,
                format!(
                    "版本冲突：远端备份已被其他设备修改，服务器: {host}",
                    host = self.host
                ),
            ));
        }
        if !status.is_success() {
            return Err(AppError::with_detail(
                ErrorCode::CloudSyncFailed,
                format!(
                    "上传备份到 WebDAV 失败，HTTP {status}，服务器: {host}",
                    host = self.host
                ),
            ));
        }
        Ok(())
    }

    async fn get_document(&self) -> Result<Vec<u8>, AppError> {
        self.get_document_versioned().await.map(|(data, _)| data)
    }

    async fn probe_document(&self) -> Result<Option<String>, AppError> {
        let response = self
            .send(reqwest::Method::HEAD, &self.object_url, None, None)
            .await?;
        let status = response.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !status.is_success() {
            return Err(AppError::with_detail(
                ErrorCode::CloudSyncFailed,
                format!(
                    "检查 WebDAV 远端对象失败，HTTP {status}，服务器: {host}",
                    host = self.host
                ),
            ));
        }
        Ok(response
            .headers()
            .get(reqwest::header::ETAG)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string))
    }

    async fn get_document_versioned(&self) -> Result<(Vec<u8>, Option<String>), AppError> {
        let response = self
            .send(reqwest::Method::GET, &self.object_url, None, None)
            .await?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(AppError::with_detail(
                ErrorCode::CloudSyncMissing,
                format!(
                    "WebDAV 端点上还没有同步备份，服务器: {host}",
                    host = self.host
                ),
            ));
        }
        let status = response.status();
        if !status.is_success() {
            return Err(AppError::with_detail(
                ErrorCode::CloudSyncFailed,
                format!(
                    "从 WebDAV 下载备份失败，HTTP {status}，服务器: {host}",
                    host = self.host
                ),
            ));
        }
        let version = response
            .headers()
            .get(reqwest::header::ETAG)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let data = download_limited(response, &self.host).await?;
        Ok((data, version))
    }
}

impl DigestChallenge {
    fn authorization(
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

fn digest_challenge(response: &reqwest::Response) -> Option<DigestChallenge> {
    response
        .headers()
        .get_all(WWW_AUTHENTICATE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find_map(parse_digest_challenge)
}

fn parse_digest_challenge(header: &str) -> Option<DigestChallenge> {
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

fn digest_uri(url: &reqwest::Url) -> String {
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

fn next_cnonce() -> String {
    let counter = CNONCE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("{nanos:x}{counter:08x}")
}

/// 网络层错误的统一包装：附带动作语境与主机名，不含凭据。
pub(super) fn http_error(action: &str, host: &str, error: reqwest::Error) -> AppError {
    let detail = if error.is_timeout() {
        "请求超时".to_string()
    } else if error.is_connect() {
        "无法建立连接".to_string()
    } else {
        error.to_string()
    };
    AppError::with_detail(
        ErrorCode::CloudSyncFailed,
        format!("{action}失败，服务器: {host}，原因: {detail}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(base: &str, directory: &str) -> Result<WebDavTarget, AppError> {
        WebDavTarget::new(
            reqwest::Client::new(),
            base,
            "user@example.com",
            Some("app-password"),
            directory,
        )
    }

    #[test]
    fn object_and_directory_urls_are_joined_level_by_level() {
        let target = target("https://dav.example.com/remote.php/dav/files/user", "a/b").unwrap();
        assert_eq!(
            target.object_url.as_str(),
            "https://dav.example.com/remote.php/dav/files/user/a/b/catalog.transombak"
        );
        assert_eq!(
            target
                .directory_urls
                .iter()
                .map(reqwest::Url::as_str)
                .collect::<Vec<_>>(),
            vec![
                "https://dav.example.com/remote.php/dav/files/user/a/",
                "https://dav.example.com/remote.php/dav/files/user/a/b/"
            ]
        );
    }

    #[test]
    fn base_query_and_fragment_are_dropped() {
        let target = target(
            "https://dav.example.com/root?token=leak#fragment",
            "transom",
        )
        .unwrap();
        let url = target.object_url.as_str();
        assert!(!url.contains("token=leak"));
        assert!(!url.contains("#fragment"));
    }

    #[test]
    fn insecure_and_non_https_schemes_are_rejected() {
        for base in ["http://dav.example.com", "ftp://dav.example.com"] {
            assert!(target(base, "t").is_err());
        }
    }

    #[test]
    fn malformed_url_is_rejected() {
        assert!(target("not a url", "t").is_err());
    }

    #[test]
    fn basic_authorization_carries_encoded_credentials() {
        let target = target("https://dav.example.com", "t").unwrap();
        let header = target
            .authorization(&reqwest::Method::GET, &target.object_url, false)
            .unwrap()
            .unwrap();
        assert_eq!(header, "Basic dXNlckBleGFtcGxlLmNvbTphcHAtcGFzc3dvcmQ=");
    }

    #[test]
    fn anonymous_endpoints_skip_authorization() {
        let target = WebDavTarget::new(
            reqwest::Client::new(),
            "https://dav.example.com",
            "",
            None,
            "t",
        )
        .unwrap();
        assert!(
            target
                .authorization(&reqwest::Method::GET, &target.object_url, false)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn digest_challenge_parses_quoted_qop_and_opaque() {
        let challenge = parse_digest_challenge(
            r#"Digest realm="dav", nonce="abc,123", algorithm=SHA-256, qop="auth,auth-int", opaque="opaque\"" "#,
        )
        .unwrap();
        assert_eq!(challenge.realm, "dav");
        assert_eq!(challenge.nonce, "abc,123");
        assert_eq!(challenge.algorithm, DigestAlgorithm::Sha256);
        assert_eq!(challenge.qop.as_deref(), Some("auth"));
        assert_eq!(challenge.opaque.as_deref(), Some("opaque\""));
    }

    #[test]
    fn digest_authorization_is_deterministic_for_fixed_inputs() {
        let challenge = DigestChallenge {
            realm: "testrealm@host.com".into(),
            nonce: "dcd98b7102dd2f0e8b11d0f600bfb0c093".into(),
            algorithm: DigestAlgorithm::Md5,
            qop: Some("auth".into()),
            opaque: Some("5ccc069c403ebaf9f0171e9517f40e41".into()),
        };
        let header = challenge
            .authorization(
                "GET",
                "/dir/index.html",
                "Mufasa",
                "Circle Of Life",
                1,
                "0a4f113b",
            )
            .unwrap();
        assert!(header.starts_with("Digest username=\"Mufasa\""));
        assert!(header.contains("response=\"6629fae49393a05397450978507c4ef1\""));
        assert!(header.contains("opaque=\"5ccc069c403ebaf9f0171e9517f40e41\""));
    }
}
