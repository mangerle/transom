//! AWS Signature Version 4 请求签名（仅覆盖 S3 对象读写所需的最小子集）。
//!
//! 设计取舍（Why）：同步只需要 PUT/GET 两个动作，签名走 header 方式并
//! 对载荷做真实 SHA256（而非预签名 URL 的 UNSIGNED-PAYLOAD），避免引入
//! 额外签名库；算法是确定性的，因此用 AWS 官方文档的两个完整示例
//! （PUT/GET）作为单元测试向量，签名实现与官方结果逐字节对齐。
//!
//! 代价与局限：不支持 streaming chunk 上传（备份体量上限 16 MiB，一次性
//! 读取内存可控）；不支持 session token（长期 IAM 凭据与自托管 MinIO 均
//! 不需要）。

use chrono::Utc;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

/// 空载荷的 SHA256；GET 请求的 `x-amz-content-sha256` 固定取该值。
pub const EMPTY_PAYLOAD_SHA256: &str =
    "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

/// 一次 SigV4 签名所需的全部输入。
///
/// 参数对象化（AGENTS.md 参数收敛）：SigV4 本就要吃方法、URI、查询、头列表、
/// 载荷摘要、时间戳、区域与一对凭据共 9 项，平铺传参既超限又容易把
/// `access_key` 与 `secret_key` 写反。字段一律借用，调用方不必克隆。
#[derive(Debug, Clone, Copy)]
pub struct SignatureRequest<'a> {
    pub method: &'a str,
    pub canonical_uri: &'a str,
    pub query: &'a str,
    pub headers: &'a [(&'a str, &'a str)],
    pub payload_hash: &'a str,
    pub amz_date: &'a str,
    pub region: &'a str,
    pub access_key: &'a str,
    pub secret_key: &'a str,
}

/// 生成 `Authorization` 头的完整取值。
///
/// `headers` 内部做小写化、去空白并按名称排序，调用方无需预排序；
/// `canonical_uri` 必须已按 S3 规则做过单次 URI 编码（见 [`uri_encode`]）。
#[must_use]
pub fn authorization_header(request: &SignatureRequest<'_>) -> String {
    let SignatureRequest {
        method,
        canonical_uri,
        query,
        headers,
        payload_hash,
        amz_date,
        region,
        access_key,
        secret_key,
    } = *request;
    let mut signed: Vec<(String, String)> = headers
        .iter()
        .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_string()))
        .collect();
    signed.sort_by(|a, b| a.0.cmp(&b.0));
    let signed_names: Vec<&str> = signed.iter().map(|(name, _)| name.as_str()).collect();
    let signed_headers = signed_names.join(";");

    let canonical_headers: String = signed
        .iter()
        .map(|(name, value)| format!("{name}:{value}\n"))
        .collect();
    let canonical_request = format!(
        "{method}\n{canonical_uri}\n{query}\n{canonical_headers}\n{signed_headers}\n{payload_hash}"
    );

    let date_scope = &amz_date[..8];
    let scope = format!("{date_scope}/{region}/s3/aws4_request");
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}",
        hex::encode(Sha256::digest(canonical_request.as_bytes()))
    );

    let signature = hex::encode(hmac_chain(secret_key, date_scope, region, &string_to_sign));
    format!(
        "AWS4-HMAC-SHA256 Credential={access_key}/{scope},SignedHeaders={signed_headers},Signature={signature}"
    )
}

/// 当前 UTC 时间的 `x-amz-date` 取值（`YYYYMMDDTHHMMSSZ`）。
#[must_use]
pub fn amz_date_now() -> String {
    Utc::now().format("%Y%m%dT%H%M%SZ").to_string()
}

/// 按 S3 单次编码规则转义路径段：保留 `[A-Za-z0-9-_.~]`，其余转 `%XX`。
///
/// S3 对路径不做二次编码（与通用 SigV4 的 double-encode 不同），因此只
/// 编码一次；`encode_slash` 控制是否转义 `/`——对象键按段编码时传 `false`。
#[must_use]
pub fn uri_encode(value: &str, encode_slash: bool) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        let keep = byte.is_ascii_alphanumeric()
            || matches!(byte, b'-' | b'_' | b'.' | b'~')
            || (byte == b'/' && !encode_slash);
        if keep {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

/// SigV4 派生链：date -> region -> service -> signing key -> 签名。
fn hmac_chain(secret_key: &str, date_scope: &str, region: &str, string_to_sign: &str) -> [u8; 32] {
    fn hmac(key: &[u8], data: &str) -> [u8; 32] {
        let mut mac = HmacSha256::new_from_slice(key).expect("HMAC 接受任意长度密钥");
        mac.update(data.as_bytes());
        mac.finalize().into_bytes().into()
    }
    let k_date = hmac(format!("AWS4{secret_key}").as_bytes(), date_scope);
    let k_region = hmac(&k_date, region);
    let k_service = hmac(&k_region, "s3");
    let k_signing = hmac(&k_service, "aws4_request");
    hmac(&k_signing, string_to_sign)
}

#[cfg(test)]
mod tests {
    use super::*;

    // AWS 官方文档示例的固定凭据，仅用于测试向量。
    const ACCESS_KEY: &str = "AKIAIOSFODNN7EXAMPLE";
    const SECRET_KEY: &str = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";

    /// AWS 官方文档 PUT 示例（分块上传的 seed signature）：签七条 header，
    /// 期望签名取自文档 "Example: PUT Object" 一节，逐字对齐。
    #[test]
    fn matches_aws_documented_put_example() {
        let headers = [
            ("Content-Encoding", "aws-chunked"),
            ("Content-Length", "66824"),
            ("Host", "s3.amazonaws.com"),
            ("X-Amz-Content-Sha256", "STREAMING-AWS4-HMAC-SHA256-PAYLOAD"),
            ("X-Amz-Date", "20130524T000000Z"),
            ("X-Amz-Decoded-Content-Length", "66560"),
            ("X-Amz-Storage-Class", "REDUCED_REDUNDANCY"),
        ];
        let authorization = authorization_header(&SignatureRequest {
            method: "PUT",
            canonical_uri: "/examplebucket/chunkObject.txt",
            query: "",
            headers: &headers,
            payload_hash: "STREAMING-AWS4-HMAC-SHA256-PAYLOAD",
            amz_date: "20130524T000000Z",
            region: "us-east-1",
            access_key: ACCESS_KEY,
            secret_key: SECRET_KEY,
        });
        assert_eq!(
            authorization,
            "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request,\
             SignedHeaders=content-encoding;content-length;host;x-amz-content-sha256;x-amz-date;\
             x-amz-decoded-content-length;x-amz-storage-class,\
             Signature=4f232c4386841ef735655705268965c44a0e4690baa4adea153f7db9fa80a0a9"
        );
    }

    /// 官方 GET 示例（带 Range 头读取前 10 字节）。
    #[test]
    fn matches_aws_documented_get_example() {
        let headers = [
            ("Range", "bytes=0-9"),
            ("Host", "examplebucket.s3.amazonaws.com"),
            ("X-Amz-Content-Sha256", EMPTY_PAYLOAD_SHA256),
            ("X-Amz-Date", "20130524T000000Z"),
        ];
        let authorization = authorization_header(&SignatureRequest {
            method: "GET",
            canonical_uri: "/test.txt",
            query: "",
            headers: &headers,
            payload_hash: EMPTY_PAYLOAD_SHA256,
            amz_date: "20130524T000000Z",
            region: "us-east-1",
            access_key: ACCESS_KEY,
            secret_key: SECRET_KEY,
        });
        assert!(authorization.ends_with(
            "Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"
        ));
        assert!(authorization.contains("SignedHeaders=host;range;x-amz-content-sha256;x-amz-date"));
    }

    #[test]
    fn uri_encode_keeps_unreserved_and_escapes_specials() {
        assert_eq!(uri_encode("catalog.transombak", true), "catalog.transombak");
        assert_eq!(uri_encode("test$file.text", false), "test%24file.text");
        assert_eq!(uri_encode("a/b", false), "a/b");
        assert_eq!(uri_encode("a/b", true), "a%2Fb");
        assert_eq!(uri_encode("空格 x", false), "%E7%A9%BA%E6%A0%BC%20x");
    }
}
