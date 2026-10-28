//! S3 兼容云同步端点：path-style 寻址 + SigV4 header 签名的最小对象读写。
//!
//! 设计说明（Why）：
//! - path-style（`{endpoint}/{bucket}/{key}`）而非虚拟主机式：自托管 MinIO
//!   默认即 path-style，且不要求 DNS 通配符证书，自托管人群零额外部署；
//! - 每次请求现场签名：备份是分钟级低频操作，不值得引入时钟漂移追踪；
//! - 载荷哈希用真实 SHA256（PUT）与空串哈希（GET），为 16 MiB 上限内的
//!   小对象设计，不涉及分块上传协议。

use reqwest::header::{AUTHORIZATION, HOST, HeaderMap, HeaderValue};
use sha2::{Digest, Sha256};
use transom_core::{AppError, CloudSyncTarget, ErrorCode, SYNC_OBJECT_NAME};
use zeroize::Zeroizing;

use super::download_limited;
use super::sigv4;
use super::webdav::http_error;

/// S3 远端目标：端点、寻址信息与签名凭据。
pub struct S3Target {
    http: reqwest::Client,
    /// 服务根地址（scheme://host[:port]）。
    endpoint: reqwest::Url,
    region: String,
    bucket: String,
    access_key: String,
    secret_key: Zeroizing<String>,
    /// 备份对象的完整键（目录段 + 固定对象名）。
    object_key: String,
    /// 供错误信息定位的服务器主机名（不含凭据）。
    host: String,
}

impl S3Target {
    /// 构造 S3 目标。
    ///
    /// # Errors
    /// 端点不是合法的 https URL，或目录键无法构造时返回 `AppError`。
    pub fn new(
        http: reqwest::Client,
        endpoint: &str,
        region: &str,
        bucket: &str,
        access_key: &str,
        secret_key: Zeroizing<String>,
        directory: &str,
    ) -> Result<Self, AppError> {
        let mut url = reqwest::Url::parse(endpoint).map_err(|error| {
            AppError::with_detail(
                ErrorCode::Configuration,
                format!("S3 端点地址无法解析: {error}"),
            )
        })?;
        url.set_query(None);
        url.set_fragment(None);
        if url.scheme() != "https" {
            return Err(AppError::with_detail(
                ErrorCode::Configuration,
                format!(
                    "S3 端点必须使用 https（http 会明文传输 Secret Key 与备份内容），当前为「{endpoint}」"
                ),
            ));
        }
        let host = url
            .host_str()
            .unwrap_or("unknown")
            .trim_end_matches('/')
            .to_string();

        // 桶名来自用户配置，不能假定它只含 URL 安全字符；先在构造期拒绝
        // 会让对象地址拼不出合法 URL 的输入，避免运行期 panic 或静默改路径。
        let bucket = bucket.trim();
        if bucket.is_empty()
            || !bucket
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '.' | '_'))
        {
            return Err(AppError::with_detail(
                ErrorCode::Configuration,
                format!("S3 存储桶名称非法: {bucket:?}，只允许字母、数字、-、.、_"),
            ));
        }

        let mut key_segments: Vec<String> = Vec::with_capacity(2);
        for segment in directory.split('/').filter(|segment| !segment.is_empty()) {
            key_segments.push(sigv4::uri_encode(segment, false));
        }
        key_segments.push(SYNC_OBJECT_NAME.to_string());
        let object_key = key_segments.join("/");

        Ok(Self {
            http,
            endpoint: url,
            region: region.to_string(),
            bucket: bucket.to_string(),
            access_key: access_key.to_string(),
            secret_key,
            object_key,
            host,
        })
    }

    /// 备份对象的完整请求地址与签名用规范 URI（两者共用同一次编码）。
    ///
    /// # Errors
    /// 端点与对象键拼接后不是合法 URL 时返回 `Configuration` 错误；构造期
    /// 已校验桶名，正常路径不会走到这里。
    fn object_url(&self) -> Result<(reqwest::Url, String), AppError> {
        // 段已在构造时完成单次编码；此处仅做路径拼接。
        let canonical_uri = format!("/{}/{}", self.bucket, self.object_key);
        let url = format!(
            "{}{}",
            self.endpoint.as_str().trim_end_matches('/'),
            canonical_uri
        );
        let url = reqwest::Url::parse(&url).map_err(|error| {
            AppError::with_detail(
                ErrorCode::Configuration,
                format!("S3 对象地址无法解析: {error}"),
            )
        })?;
        Ok((url, canonical_uri))
    }

    /// 签名所需的 host 头取值：含非标准端口，不含路径。
    fn host_header(&self) -> String {
        let port = self.endpoint.port();
        match (port, self.endpoint.scheme()) {
            (Some(port), "http") if port != 80 => format!("{host}:{port}", host = self.host),
            (Some(port), "https") if port != 443 => format!("{host}:{port}", host = self.host),
            _ => self.host.clone(),
        }
    }

    async fn send_signed(
        &self,
        method: reqwest::Method,
        data: Option<Vec<u8>>,
        if_match: Option<&str>,
    ) -> Result<reqwest::Response, AppError> {
        let (url, canonical_uri) = self.object_url()?;
        let payload_hash = match &data {
            Some(data) => hex::encode(Sha256::digest(data)),
            None => sigv4::EMPTY_PAYLOAD_SHA256.to_string(),
        };
        let amz_date = sigv4::amz_date_now();
        let host_header = self.host_header();
        let mut headers = HeaderMap::with_capacity(4);
        headers.insert(
            HOST,
            HeaderValue::from_str(&host_header).map_err(|_| {
                AppError::with_detail(ErrorCode::Configuration, "S3 端点主机名不合法")
            })?,
        );
        headers.insert(
            reqwest::header::HeaderName::from_static("x-amz-content-sha256"),
            HeaderValue::from_str(&payload_hash)
                .map_err(|_| AppError::with_detail(ErrorCode::Configuration, "载荷哈希不合法"))?,
        );
        headers.insert(
            reqwest::header::HeaderName::from_static("x-amz-date"),
            HeaderValue::from_str(&amz_date)
                .map_err(|_| AppError::with_detail(ErrorCode::Configuration, "签名时间戳不合法"))?,
        );
        if let Some(version) = if_match {
            headers.insert(
                reqwest::header::IF_MATCH,
                HeaderValue::from_str(version).map_err(|_| {
                    AppError::with_detail(
                        ErrorCode::Configuration,
                        "远端版本标识包含非法字符，无法作为 If-Match 发送",
                    )
                })?,
            );
        }
        // 签名头列表与发送头列表必须一致：If-Match 参与签名，否则代理可篡改
        // 前置条件让并发写互相覆盖。
        let mut signed: Vec<(&str, &str)> = vec![
            ("host", host_header.as_str()),
            ("x-amz-content-sha256", payload_hash.as_str()),
            ("x-amz-date", amz_date.as_str()),
        ];
        if let Some(version) = if_match {
            signed.push(("if-match", version));
        }
        let authorization = sigv4::authorization_header(&sigv4::SignatureRequest {
            method: method.as_str(),
            canonical_uri: &canonical_uri,
            query: "",
            headers: &signed,
            payload_hash: &payload_hash,
            amz_date: &amz_date,
            region: &self.region,
            access_key: &self.access_key,
            secret_key: self.secret_key.as_str(),
        });

        let mut builder = self
            .http
            .request(method, url)
            .headers(headers)
            .header(AUTHORIZATION, authorization);
        if let Some(data) = data {
            builder = builder.body(data);
        }
        builder
            .send()
            .await
            .map_err(|error| http_error("访问 S3 端点", &self.host, error))
    }
}

#[async_trait::async_trait]
impl CloudSyncTarget for S3Target {
    async fn put_document(&self, data: &[u8]) -> Result<(), AppError> {
        self.put_document_if_match(data, None).await
    }

    async fn put_document_if_match(
        &self,
        data: &[u8],
        version: Option<&str>,
    ) -> Result<(), AppError> {
        let response = self
            .send_signed(reqwest::Method::PUT, Some(data.to_vec()), version)
            .await?;
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
                    "上传备份到 S3 失败，HTTP {status}，服务器: {host}",
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
        let response = self.send_signed(reqwest::Method::GET, None, None).await?;
        if response.status() == reqwest::StatusCode::NOT_FOUND
            || response.status() == reqwest::StatusCode::NO_CONTENT
        {
            return Err(AppError::with_detail(
                ErrorCode::CloudSyncMissing,
                format!("S3 端点上还没有同步备份，服务器: {host}", host = self.host),
            ));
        }
        let status = response.status();
        if !status.is_success() {
            return Err(AppError::with_detail(
                ErrorCode::CloudSyncFailed,
                format!(
                    "从 S3 下载备份失败，HTTP {status}，服务器: {host}",
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

#[cfg(test)]
mod tests {
    use super::*;

    fn target() -> S3Target {
        S3Target::new(
            reqwest::Client::new(),
            "https://s3.example.com:9000",
            "us-east-1",
            "transom-vault",
            "AKIDEXAMPLE",
            Zeroizing::new("secret".to_string()),
            "backups",
        )
        .unwrap()
    }

    #[test]
    fn object_url_uses_path_style_addressing() {
        let (url, canonical_uri) = target().object_url().expect("对象地址应可解析");
        assert_eq!(
            url.as_str(),
            "https://s3.example.com:9000/transom-vault/backups/catalog.transombak"
        );
        assert_eq!(canonical_uri, "/transom-vault/backups/catalog.transombak");
    }

    #[test]
    fn invalid_bucket_is_rejected_instead_of_panicking() {
        for bucket in ["", "bad bucket", "bucket?x", "bucket#frag", "a/b"] {
            let error = S3Target::new(
                reqwest::Client::new(),
                "https://s3.example.com",
                "us-east-1",
                bucket,
                "AKID",
                Zeroizing::new("secret".to_string()),
                "t",
            )
            .err()
            .expect("非法桶名应在构造期失败");
            assert_eq!(error.code, ErrorCode::Configuration);
        }
    }

    #[test]
    fn host_header_keeps_nonstandard_port() {
        assert_eq!(target().host_header(), "s3.example.com:9000");

        let standard = S3Target::new(
            reqwest::Client::new(),
            "https://s3.amazonaws.com",
            "us-east-1",
            "bucket",
            "AKID",
            Zeroizing::new("secret".to_string()),
            "t",
        )
        .unwrap();
        assert_eq!(standard.host_header(), "s3.amazonaws.com");
    }

    #[test]
    fn insecure_and_non_https_endpoints_are_rejected() {
        for endpoint in ["http://s3.example.com", "ftp://s3.example.com"] {
            let error = S3Target::new(
                reqwest::Client::new(),
                endpoint,
                "us-east-1",
                "bucket",
                "AKID",
                Zeroizing::new("secret".to_string()),
                "t",
            )
            .err()
            .expect("预期构造失败");
            assert_eq!(error.code, ErrorCode::Configuration);
        }
    }
}
