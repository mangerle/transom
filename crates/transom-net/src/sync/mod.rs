//! 云端同步：端点传输（WebDAV/S3）、S3 请求签名与加密备份的推送拉取编排。

mod git;
mod s3;
mod service;
mod sigv4;
mod webdav;

pub use git::{GIT_SYNC_FILE, GitAuth, GitSyncOutcome, sync_git_blocking};
pub use service::CloudSyncService;

use transom_core::{AppError, ErrorCode, SYNC_MAX_DOCUMENT_BYTES};
use webdav::http_error;

/// 流式读取响应体并强制大小上限，杜绝把不可信端点的超大响应整体读入内存。
///
/// 响应头声明的大小先做快速拦截，随后分块累计计数兜底（Content-Length
/// 可能缺失或被中间设备改写）。
async fn download_limited(response: reqwest::Response, host: &str) -> Result<Vec<u8>, AppError> {
    if let Some(length) = response.content_length()
        && length > SYNC_MAX_DOCUMENT_BYTES
    {
        return Err(too_large(host, length));
    }
    let mut buffer = Vec::with_capacity(
        response
            .content_length()
            .unwrap_or(64 * 1024)
            .min(SYNC_MAX_DOCUMENT_BYTES) as usize,
    );
    let mut response = response;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| http_error("从云端同步端点读取备份", host, error))?
    {
        if (buffer.len() as u64).saturating_add(chunk.len() as u64) > SYNC_MAX_DOCUMENT_BYTES {
            return Err(too_large(host, buffer.len() as u64 + chunk.len() as u64));
        }
        buffer.extend_from_slice(&chunk);
    }
    Ok(buffer)
}

fn too_large(host: &str, length: u64) -> AppError {
    AppError::with_detail(
        ErrorCode::CloudSyncFailed,
        format!(
            "云端备份超过 {SYNC_MAX_DOCUMENT_BYTES} 字节上限，已拒绝读取，服务器: {host}，实际: {length} 字节"
        ),
    )
}
