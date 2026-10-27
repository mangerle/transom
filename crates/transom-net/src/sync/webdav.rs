//! WebDAV 云同步端点：坚果云、Nextcloud 等自托管网盘的最小 PUT/GET 客户端。
//!
//! 设计说明（Why）：
//! - 只依赖 `PUT`/`GET`/`MKCOL` 三个方法：同步协议收敛为单对象全量读写，
//!   PROPFIND 列举等能力一概不用，端点兼容面最小（坚果云对部分 WebDAV
//!   方法有配额限制，方法越少越稳）；
//! - 目录创建采用"先 PUT 后补救"：绝大多数请求路径上目录已存在，直接
//!   PUT 命中零额外往返；仅当服务器返回 404/409 时才逐级 MKCOL 后重试。

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use transom_core::{AppError, CloudSyncTarget, ErrorCode, SYNC_OBJECT_NAME};
use zeroize::Zeroizing;

use super::download_limited;

/// WebDAV 远端目标：固定对象的上传下载地址与逐级目录地址。
pub struct WebDavTarget {
    http: reqwest::Client,
    object_url: reqwest::Url,
    /// 需要按序创建的目录地址（不含端点根目录本身）。
    directory_urls: Vec<reqwest::Url>,
    /// Basic 认证头；无密码的匿名端点为 `None`。
    authorization: Option<Zeroizing<String>>,
    /// 供错误信息定位的服务器主机名（不含凭据）。
    host: String,
}

impl WebDavTarget {
    /// 构造 WebDAV 目标。
    ///
    /// # Errors
    /// 根地址不是合法的 http(s) URL，或目录段无法拼接时返回 `AppError`。
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
        if !matches!(base.scheme(), "http" | "https") {
            return Err(AppError::with_detail(
                ErrorCode::Configuration,
                format!("WebDAV 地址必须以 http(s) 开头，当前为「{base_url}」"),
            ));
        }
        let host = base
            .host_str()
            .unwrap_or("unknown")
            .trim_end_matches('/')
            .to_string();

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

        let authorization = password.map(|password| {
            let token = BASE64.encode(format!("{username}:{password}"));
            Zeroizing::new(format!("Basic {token}"))
        });
        Ok(Self {
            http,
            object_url,
            directory_urls,
            authorization,
            host,
        })
    }

    fn request(&self, method: reqwest::Method, url: &reqwest::Url) -> reqwest::RequestBuilder {
        let mut builder = self.http.request(method.clone(), url.clone());
        if let Some(authorization) = &self.authorization {
            builder = builder.header(reqwest::header::AUTHORIZATION, authorization.as_str());
        }
        builder
    }

    /// 逐级创建目录；已存在的目录（405）视为成功。
    ///
    /// MKCOL 一次只建一级，逐级推进时同级失败立即中断，错误信息带上
    /// 主机名便于定位，绝不回显凭据。
    async fn ensure_directories(&self) -> Result<(), AppError> {
        for url in &self.directory_urls {
            let response = self
                .request(
                    reqwest::Method::from_bytes(b"MKCOL").expect("MKCOL 是合法方法名"),
                    url,
                )
                .send()
                .await
                .map_err(|error| http_error("创建 WebDAV 目录", &self.host, error))?;
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
        let mut request = self.request(reqwest::Method::PUT, &self.object_url);
        if let Some(version) = version {
            request = request.header(reqwest::header::IF_MATCH, version);
        }
        let mut response = request
            .body(data.to_vec())
            .send()
            .await
            .map_err(|error| http_error("上传备份到 WebDAV", &self.host, error))?;
        // 目录缺失在部分实现上表现为 404（路径不存在），部分为 409（父级缺失）。
        if response.status() == reqwest::StatusCode::NOT_FOUND
            || response.status() == reqwest::StatusCode::CONFLICT
        {
            response = {
                self.ensure_directories().await?;
                let mut request = self.request(reqwest::Method::PUT, &self.object_url);
                if let Some(version) = version {
                    request = request.header(reqwest::header::IF_MATCH, version);
                }
                request
                    .body(data.to_vec())
                    .send()
                    .await
                    .map_err(|error| http_error("上传备份到 WebDAV", &self.host, error))?
            };
        }
        let status = response.status();
        if status == reqwest::StatusCode::PRECONDITION_FAILED {
            return Err(AppError::with_detail(
                ErrorCode::CloudSyncFailed,
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

    async fn get_document_versioned(&self) -> Result<(Vec<u8>, Option<String>), AppError> {
        let response = self
            .request(reqwest::Method::GET, &self.object_url)
            .send()
            .await
            .map_err(|error| http_error("从 WebDAV 下载备份", &self.host, error))?;
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
        let target = target("https://dav.jianguoyun.com/dav/", "transom/sessions").unwrap();
        assert_eq!(
            target.object_url.as_str(),
            "https://dav.jianguoyun.com/dav/transom/sessions/catalog.transombak"
        );
        assert_eq!(target.directory_urls.len(), 2);
        assert_eq!(
            target.directory_urls[0].as_str(),
            "https://dav.jianguoyun.com/dav/transom/"
        );
        assert_eq!(
            target.directory_urls[1].as_str(),
            "https://dav.jianguoyun.com/dav/transom/sessions/"
        );
    }

    #[test]
    fn base_query_and_fragment_are_dropped() {
        let target = target("https://dav.example.com/dav/?token=abc#frag", "t").unwrap();
        assert_eq!(
            target.object_url.as_str(),
            "https://dav.example.com/dav/t/catalog.transombak"
        );
    }

    #[test]
    fn non_http_scheme_is_rejected() {
        let error = target("ftp://dav.example.com", "t")
            .err()
            .expect("预期构造失败");
        assert_eq!(error.code, ErrorCode::Configuration);
    }

    #[test]
    fn malformed_url_is_rejected() {
        let error = target("not a url", "t").err().expect("预期构造失败");
        assert_eq!(error.code, ErrorCode::Configuration);
    }

    #[test]
    fn authorization_header_carries_basic_token() {
        let target = target("https://dav.example.com", "t").unwrap();
        let token = target.authorization.unwrap();
        let expected = BASE64.encode("user@example.com:app-password");
        assert_eq!(token.as_str(), format!("Basic {expected}"));
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
        assert!(target.authorization.is_none());
    }
}
