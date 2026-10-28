//! SFTP 文件 SHA256 校验：本地后台线程分块计算，远端通过 SFTP 流式读取。

use crate::sftp::{SftpClient, sftp_error, sftp_io_error};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};
use tokio::io::AsyncReadExt;
use transom_core::{AppError, ErrorCode};

const HASH_CHUNK: usize = 64 * 1024;

impl SftpClient {
    /// 流式读取远端文件并计算 SHA256，不会把整个文件读入内存。
    ///
    /// # Errors
    /// 远端文件不存在、读取中断或会话异常时返回 `ErrorCode::SftpFailed`。
    pub async fn sha256_remote(&self, path: &str) -> Result<String, AppError> {
        let mut remote = self.session.open(path).await.map_err(sftp_error)?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0u8; HASH_CHUNK];
        loop {
            let count = remote.read(&mut buffer).await.map_err(sftp_io_error)?;
            if count == 0 {
                break;
            }
            hasher.update(&buffer[..count]);
        }
        let _ = remote.close().await;
        Ok(hex_encode(&hasher.finalize()))
    }

    /// 并行计算本地与远端 SHA256 并比对，支持复用在途计算的本地哈希以消除二次读盘。
    ///
    /// 设计原理（Why）：在单文件传输流中，数据已完整流经本地内存缓冲区；
    /// 复用在途计算的哈希可彻底消灭传输完成后对本地磁盘的全量重复读取。若在途哈希缺失
    /// （如断点续传 offset > 0），则安全回退到本地读盘计算。
    ///
    /// # Errors
    /// 读取失败或两侧哈希不一致时返回 `ErrorCode::SftpFailed`。
    pub async fn verify_file_with_known_local(
        &self,
        local_path: &Path,
        remote_path: &str,
        known_local: Option<&str>,
    ) -> Result<(), AppError> {
        let (local_hash, remote_hash) = match known_local {
            Some(known) => (Ok(known.to_string()), self.sha256_remote(remote_path).await),
            None => tokio::join!(sha256_local(local_path), self.sha256_remote(remote_path)),
        };
        let local_hash = local_hash?;
        let remote_hash = remote_hash?;
        if local_hash == remote_hash {
            Ok(())
        } else {
            Err(AppError::with_detail(
                ErrorCode::SftpFailed,
                format!(
                    "SHA256 校验不一致：本地 {}（{local_hash}），远端 {}（{remote_hash}）",
                    local_path.display(),
                    remote_path
                ),
            ))
        }
    }

    /// 并行计算本地与远端 SHA256 并比对。
    ///
    /// 设计原理（Why）：本端读盘与远端读取互不依赖，使用 `tokio::join!`
    /// 让两者并行，校验总耗时接近较慢的一侧而非两者之和；本地读取通过
    /// `spawn_blocking` 移出异步工作线程，避免大文件哈希阻塞运行时。
    ///
    /// # Errors
    /// 任一侧读取失败或两侧哈希不一致时返回 `ErrorCode::SftpFailed`；
    /// 不一致详情包含两端哈希与路径，便于定位损坏文件。
    pub async fn verify_file(&self, local_path: &Path, remote_path: &str) -> Result<(), AppError> {
        self.verify_file_with_known_local(local_path, remote_path, None)
            .await
    }
}

pub(crate) async fn sha256_local(path: &Path) -> Result<String, AppError> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || hash_local_blocking(&path))
        .await
        .map_err(|error| {
            AppError::with_detail(
                ErrorCode::SftpFailed,
                format!("本地 SHA256 计算任务异常结束: {error}"),
            )
        })?
}

fn hash_local_blocking(path: &PathBuf) -> Result<String, AppError> {
    let mut file = std::fs::File::open(path).map_err(|error| {
        AppError::with_detail(
            ErrorCode::SftpFailed,
            format!("打开本地文件失败 {}: {error}", path.display()),
        )
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; HASH_CHUNK];
    loop {
        let count = file.read(&mut buffer).map_err(|error| {
            AppError::with_detail(
                ErrorCode::SftpFailed,
                format!("读取本地文件失败 {}: {error}", path.display()),
            )
        })?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(hex_encode(&hasher.finalize()))
}

pub(crate) fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{hex_encode, sha256_local};

    #[tokio::test]
    async fn local_sha256_matches_known_digests() {
        let dir = tempfile::tempdir().expect("创建临时目录");
        let hello = dir.path().join("hello.txt");
        tokio::fs::write(&hello, b"hello")
            .await
            .expect("写入测试文件");
        assert_eq!(
            sha256_local(&hello).await.expect("计算 hello 哈希"),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );

        let empty = dir.path().join("empty.txt");
        tokio::fs::write(&empty, b"").await.expect("写入空文件");
        assert_eq!(
            sha256_local(&empty).await.expect("计算空文件哈希"),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn hex_encode_uses_lowercase_and_pads_bytes() {
        assert_eq!(hex_encode(&[0x00, 0x0f, 0xff]), "000fff");
    }
}
