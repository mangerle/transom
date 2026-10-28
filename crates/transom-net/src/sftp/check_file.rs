//! SFTP check-file 协议扩展：符合 IETF draft-ietf-secsh-filexfer-extensions 规范。
//!
//! 提供针对 SFTP 服务端的 check-file-name / check-file-handle 扩展协议编解码，
//! 支持客户端直接向服务端请求在远端本地计算哈希摘要，免除网络回传开销。

use transom_core::{AppError, ErrorCode};

/// IETF draft 定义的通过文件名查询校验和的扩展名称。
pub const CHECK_FILE_NAME_EXT: &str = "check-file-name";

/// IETF draft 定义的通过文件句柄查询校验和的扩展名称。
pub const CHECK_FILE_HANDLE_EXT: &str = "check-file-handle";

/// check-file 扩展请求参数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckFileRequest<'a> {
    /// 目标文件路径或句柄。
    pub target: &'a str,
    /// 请求的哈希算法名称，如 "sha256"、"md5"。
    pub algorithm: &'a str,
    /// 校验起始偏移量（字节），全量校验填 0。
    pub offset: u64,
    /// 校验长度（字节），0 表示计算从 offset 到文件结尾的所有数据。
    pub length: u64,
    /// 分块校验大小（字节），0 表示对整个范围计算单一哈希。
    pub block_size: u32,
}

impl<'a> CheckFileRequest<'a> {
    /// 构造全文件 SHA-256 校验请求。
    pub fn sha256_whole_file(target: &'a str) -> Self {
        Self {
            target,
            algorithm: "sha256",
            offset: 0,
            length: 0,
            block_size: 0,
        }
    }

    /// 将请求编码为符合 SSH 扩展包规范的二进制载荷。
    pub fn encode(&self) -> Vec<u8> {
        let mut buffer =
            Vec::with_capacity(4 + self.target.len() + 4 + self.algorithm.len() + 8 + 8 + 4);
        put_string(&mut buffer, self.target);
        put_string(&mut buffer, self.algorithm);
        buffer.extend_from_slice(&self.offset.to_be_bytes());
        buffer.extend_from_slice(&self.length.to_be_bytes());
        buffer.extend_from_slice(&self.block_size.to_be_bytes());
        buffer
    }
}

/// check-file 扩展回复响应。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckFileResponse {
    /// 服务端实际采用的算法名称。
    pub algorithm: String,
    /// 二进制哈希摘要转换为十六进制小写字符串。
    pub digest_hex: String,
}

/// 解码服务端在 `SSH_FXP_EXTENDED_REPLY` 中返回的扩展载荷。
///
/// # Errors
/// 载荷截断、字符编码非法或格式不符合规范时返回 `ErrorCode::SftpFailed`。
pub fn decode_check_file_reply(payload: &[u8]) -> Result<CheckFileResponse, AppError> {
    let mut cursor = 0;
    let algorithm = get_string(payload, &mut cursor)?;
    if cursor >= payload.len() {
        return Err(AppError::with_detail(
            ErrorCode::SftpFailed,
            "check-file 响应数据截断：缺少哈希摘要内容",
        ));
    }
    let digest_bytes = &payload[cursor..];
    let digest_hex = super::hash::hex_encode(digest_bytes);
    Ok(CheckFileResponse {
        algorithm,
        digest_hex,
    })
}

fn put_string(buffer: &mut Vec<u8>, value: &str) {
    let len = u32::try_from(value.len()).unwrap_or(u32::MAX);
    buffer.extend_from_slice(&len.to_be_bytes());
    buffer.extend_from_slice(value.as_bytes());
}

fn get_string(payload: &[u8], cursor: &mut usize) -> Result<String, AppError> {
    if payload.len().saturating_sub(*cursor) < 4 {
        return Err(AppError::with_detail(
            ErrorCode::SftpFailed,
            "解析 check-file 响应失败：长度字段截断",
        ));
    }
    let len_bytes: [u8; 4] = payload[*cursor..*cursor + 4]
        .try_into()
        .map_err(|_| AppError::with_detail(ErrorCode::SftpFailed, "读取字符串长度失败"))?;
    *cursor = cursor.saturating_add(4);
    let str_len = u32::from_be_bytes(len_bytes) as usize;
    if payload.len().saturating_sub(*cursor) < str_len {
        return Err(AppError::with_detail(
            ErrorCode::SftpFailed,
            "解析 check-file 响应失败：文本内容不足",
        ));
    }
    let slice = &payload[*cursor..*cursor + str_len];
    *cursor = cursor.saturating_add(str_len);
    String::from_utf8(slice.to_vec()).map_err(|error| {
        AppError::with_detail(
            ErrorCode::SftpFailed,
            format!("check-file 响应包含非 UTF-8 文本: {error}"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_check_file_request_matches_wire_format() {
        let req = CheckFileRequest::sha256_whole_file("/var/log/app.log");
        let encoded = req.encode();

        let mut expected = Vec::new();
        put_string(&mut expected, "/var/log/app.log");
        put_string(&mut expected, "sha256");
        expected.extend_from_slice(&0u64.to_be_bytes());
        expected.extend_from_slice(&0u64.to_be_bytes());
        expected.extend_from_slice(&0u32.to_be_bytes());

        assert_eq!(encoded, expected);
    }

    #[test]
    fn decode_check_file_reply_extracts_algorithm_and_digest() {
        let mut payload = Vec::new();
        put_string(&mut payload, "sha256");
        let sample_digest = [0xabu8, 0xcd, 0xef, 0x01, 0x23];
        payload.extend_from_slice(&sample_digest);

        let decoded = decode_check_file_reply(&payload).expect("解码成功");
        assert_eq!(decoded.algorithm, "sha256");
        assert_eq!(decoded.digest_hex, "abcdef0123");
    }

    #[test]
    fn decode_truncated_payload_fails_gracefully() {
        let truncated = vec![0, 0, 0];
        assert!(decode_check_file_reply(&truncated).is_err());

        let mut missing_digest = Vec::new();
        put_string(&mut missing_digest, "sha256");
        assert!(decode_check_file_reply(&missing_digest).is_err());
    }
}
