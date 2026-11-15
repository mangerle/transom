//! WebDAV 云同步端点：Basic/Digest 认证与最小 PUT/GET/HEAD/MKCOL 客户端。
//!
//! 设计说明（Why）：
//! - 同步协议只依赖固定对象，不枚举目录；PUT/GET/HEAD 与按需 MKCOL 已覆盖
//!   坚果云、Nextcloud 和常见 NAS 的必要能力；
//! - 先发送 Basic，收到 Digest 挑战后缓存并在后续请求预认证；这样兼容两类
//!   服务，又避免每次 PUT 都先上传一次必然失败的正文；
//! - Digest 只支持 `auth` qop 与 MD5/SHA-256 系列，不支持需要正文摘要的
//!   `auth-int`；遇到该能力时返回明确错误，而不是发送无法验证的请求。

mod digest;
#[cfg(test)]
mod tests;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use digest::{DigestState, digest_challenge, digest_uri, next_cnonce};
use parking_lot::Mutex;
use reqwest::header::AUTHORIZATION;
use transom_core::{AppError, CloudSyncTarget, ErrorCode, SYNC_OBJECT_NAME};
use zeroize::Zeroizing;

use super::download_limited;

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
