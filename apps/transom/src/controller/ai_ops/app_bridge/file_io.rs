//! AI 软件桥接的本机文件有界读取。

use std::path::Path;

use tokio::io::AsyncReadExt;
use transom_ai::ToolError;

/// 读取文件开头最多 `max_bytes` 字节。
pub(super) async fn read_prefix(
    path: &Path,
    max_bytes: usize,
    tool: &'static str,
) -> Result<Vec<u8>, ToolError> {
    let file = tokio::fs::File::open(path)
        .await
        .map_err(|error| ToolError::ExecutionFailed {
            tool,
            reason: format!("打开本机文件失败（{}）: {error}", path.display()),
        })?;
    let mut bytes = Vec::with_capacity(max_bytes.min(64 * 1024));
    file.take(max_bytes as u64)
        .read_to_end(&mut bytes)
        .await
        .map_err(|error| ToolError::ExecutionFailed {
            tool,
            reason: format!("读取本机文件失败（{}）: {error}", path.display()),
        })?;
    Ok(bytes)
}

/// 读取文件开头最多 `max_bytes` 字节，并返回是否因文件更大而截断。
pub(super) async fn read_text_prefix(
    path: &Path,
    max_bytes: usize,
    tool: &'static str,
) -> Result<(String, bool), ToolError> {
    let metadata = tokio::fs::metadata(path)
        .await
        .map_err(|error| ToolError::ExecutionFailed {
            tool,
            reason: format!("读取本机文件属性失败（{}）: {error}", path.display()),
        })?;
    let bytes = read_prefix(path, max_bytes, tool).await?;
    let mut end = bytes.len();
    while end > 0 && std::str::from_utf8(&bytes[..end]).is_err() {
        end -= 1;
    }
    Ok((
        String::from_utf8_lossy(&bytes[..end]).into_owned(),
        metadata.len() > max_bytes as u64,
    ))
}

#[cfg(test)]
mod tests {
    use super::read_text_prefix;

    #[tokio::test]
    async fn reads_only_the_bounded_prefix() {
        let dir = tempfile::tempdir().expect("创建临时目录失败");
        let path = dir.path().join("large.txt");
        tokio::fs::write(&path, "abcdefghij")
            .await
            .expect("写入测试文件失败");
        let (text, truncated) = read_text_prefix(&path, 4, "remote_fs")
            .await
            .expect("读取前缀失败");
        assert_eq!(text, "abcd");
        assert!(truncated);
    }
}
