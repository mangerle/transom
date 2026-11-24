//! AWS Signature Version 4 签名实现。

use super::ApiError;
use chrono::{DateTime, Utc};
use hmac::{Hmac, KeyInit, Mac};
use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};
use reqwest::Url;
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

/// AWS SigV4 的凭据、目标服务与签名时间。
pub(crate) struct AwsSigV4Params<'a> {
    pub(crate) access_key: &'a str,
    pub(crate) secret_key: &'a str,
    pub(crate) session_token: Option<&'a str>,
    pub(crate) region: &'a str,
    pub(crate) service: &'a str,
    pub(crate) now: DateTime<Utc>,
}

const AWS_ENCODE_SET: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'!')
    .add(b'"')
    .add(b'#')
    .add(b'$')
    .add(b'%')
    .add(b'&')
    .add(b'\'')
    .add(b'(')
    .add(b')')
    .add(b'*')
    .add(b'+')
    .add(b',')
    .add(b'/')
    .add(b':')
    .add(b';')
    .add(b'=')
    .add(b'?')
    .add(b'@')
    .add(b'[')
    .add(b']');

/// 对请求头执行 AWS SigV4 签名。
///
/// 设计原理（Why）：签名输入只使用方法、URL、头与请求体，不依赖 UI 或网络
/// 客户端；这样签名结果可做确定性单元测试，也不会把 Secret Access Key 写入
/// 请求日志。调用方必须先把凭据从环境变量解析出来，并通过 `Zeroizing` 等
/// 方式控制生命周期。
///
/// # Errors
/// 主机缺失、时间格式化或 HMAC 初始化失败时返回带上下文的中文错误。
pub(crate) fn sign_aws_v4(
    method: &str,
    url: &Url,
    headers: &mut Vec<(String, String)>,
    body: &[u8],
    params: &AwsSigV4Params<'_>,
) -> Result<(), ApiError> {
    if params.access_key.is_empty()
        || params.secret_key.is_empty()
        || params.region.is_empty()
        || params.service.is_empty()
    {
        return Err(ApiError::InvalidAuth {
            reason: "AWS 访问密钥、区域和服务不能为空".to_string(),
        });
    }
    headers.retain(|(name, _)| !name.eq_ignore_ascii_case("authorization"));
    let host = url.host_str().ok_or_else(|| ApiError::InvalidUrl {
        host: safe_host(url),
        reason: "AWS 签名要求 URL 包含主机".to_string(),
    })?;
    let host = match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_string(),
    };
    let date = params.now.format("%Y%m%dT%H%M%SZ").to_string();
    let short_date = params.now.format("%Y%m%d").to_string();
    headers.push(("host".to_string(), host));
    headers.push(("x-amz-date".to_string(), date.clone()));
    if let Some(token) = params.session_token.filter(|token| !token.is_empty()) {
        headers.push(("x-amz-security-token".to_string(), token.to_string()));
    }
    let canonical_headers = canonical_headers(headers);
    let signed_headers = canonical_headers
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>()
        .join(";");
    let canonical_query = canonical_query(url);
    let canonical_uri = if url.path().is_empty() {
        "/"
    } else {
        url.path()
    };
    let payload_hash = hex::encode(Sha256::digest(body));
    let canonical_request = format!(
        "{method}\n{canonical_uri}\n{canonical_query}\n{canonical_headers_text}\n\n{signed_headers}\n{payload_hash}",
        canonical_headers_text = canonical_headers
            .iter()
            .map(|(name, value)| format!("{name}:{value}"))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    let credential_scope = format!(
        "{short_date}/{region}/{service}/aws4_request",
        region = params.region,
        service = params.service
    );
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{date}\n{credential_scope}\n{}",
        hex::encode(Sha256::digest(canonical_request.as_bytes()))
    );
    let signing_key = signing_key(
        params.secret_key,
        &short_date,
        params.region,
        params.service,
    )?;
    let signature = hmac_hex(&signing_key, string_to_sign.as_bytes())?;
    headers.push((
        "authorization".to_string(),
        format!(
            "AWS4-HMAC-SHA256 Credential={}/{credential_scope}, SignedHeaders={signed_headers}, Signature={signature}",
            params.access_key
        ),
    ));
    Ok(())
}

fn canonical_headers(headers: &[(String, String)]) -> Vec<(String, String)> {
    let mut values = std::collections::BTreeMap::<String, Vec<String>>::new();
    for (name, value) in headers {
        let name = name.trim().to_ascii_lowercase();
        if name.is_empty() || name == "authorization" {
            continue;
        }
        values
            .entry(name)
            .or_default()
            .push(value.split_whitespace().collect::<Vec<_>>().join(" "));
    }
    values
        .into_iter()
        .map(|(name, values)| (name, values.join(",")))
        .collect()
}

fn canonical_query(url: &Url) -> String {
    let mut pairs = url
        .query_pairs()
        .map(|(key, value)| {
            (
                utf8_percent_encode(&key, AWS_ENCODE_SET).to_string(),
                utf8_percent_encode(&value, AWS_ENCODE_SET).to_string(),
            )
        })
        .collect::<Vec<_>>();
    pairs.sort();
    pairs
        .into_iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("&")
}

fn signing_key(
    secret_key: &str,
    date: &str,
    region: &str,
    service: &str,
) -> Result<Vec<u8>, ApiError> {
    let date_key = hmac_bytes(format!("AWS4{secret_key}").as_bytes(), date.as_bytes())?;
    let region_key = hmac_bytes(&date_key, region.as_bytes())?;
    let service_key = hmac_bytes(&region_key, service.as_bytes())?;
    hmac_bytes(&service_key, b"aws4_request")
}

fn hmac_bytes(key: &[u8], value: &[u8]) -> Result<Vec<u8>, ApiError> {
    let mut mac = HmacSha256::new_from_slice(key).map_err(|error| ApiError::InvalidAuth {
        reason: format!("初始化 AWS 签名密钥失败: {error}"),
    })?;
    mac.update(value);
    Ok(mac.finalize().into_bytes().to_vec())
}

fn hmac_hex(key: &[u8], value: &[u8]) -> Result<String, ApiError> {
    Ok(hex::encode(hmac_bytes(key, value)?))
}

fn safe_host(url: &Url) -> String {
    url.host_str().unwrap_or("未知主机").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn canonical_query_sorts_and_encodes() {
        let url = Url::parse("https://example.com/path?z=last&a=hello world").expect("URL");
        assert_eq!(canonical_query(&url), "a=hello%20world&z=last");
    }

    #[test]
    fn signature_is_deterministic_for_fixed_time() {
        let url = Url::parse("https://example.com/path").expect("URL");
        let now = Utc
            .with_ymd_and_hms(2024, 1, 1, 0, 0, 0)
            .single()
            .expect("时间");
        let mut headers = vec![("content-type".to_string(), "application/json".to_string())];
        sign_aws_v4(
            "GET",
            &url,
            &mut headers,
            b"",
            &AwsSigV4Params {
                access_key: "AKID",
                secret_key: "SECRET",
                session_token: None,
                region: "us-east-1",
                service: "execute-api",
                now,
            },
        )
        .expect("签名成功");
        assert!(
            headers
                .iter()
                .any(|(name, value)| name == "authorization" && value.contains("Signature="))
        );
    }
}
