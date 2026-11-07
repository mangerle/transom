//! 远端与本地文件摘要校验。
//!
//! 设计原理（Why）：远端优先走 check-file 扩展或服务端 sha256sum/md5sum，
//! 避免为校验重新下载整文件；扩展与命令都不可用时才降级为 SFTP 流式读取。
//! 本地分块哈希放在 local.rs，校验策略与已知摘要比对放在 verify.rs。

mod local;
mod verify;

use crate::keys::shell_single_quote;
use crate::sftp::{SftpClient, sftp_error, sftp_io_error};
pub(crate) use local::hex_encode;
use local::parse_hash_output;
use md5::Md5;
use sha2::{Digest, Sha256};
use std::io::SeekFrom;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use transom_core::{AppError, ChecksumAlgorithm};

/// 快速抽样校验时单段读取的字节数（64 KiB）。
pub(crate) const SAMPLE_CHUNK: usize = 64 * 1024;

/// 流式哈希计算时的默认分块缓冲区大小（256 KiB）。
///
/// 设计原理（Why）：与传输层分块保持对齐，减少系统调用并匹配预读窗口；
/// 256 KiB 的常数级开销不会引起内存突增。
pub(crate) const HASH_CHUNK: usize = 256 * 1024;

impl SftpClient {
    pub async fn checksum_remote(
        &self,
        path: &str,
        algorithm: ChecksumAlgorithm,
    ) -> Result<String, AppError> {
        match algorithm {
            ChecksumAlgorithm::Sha256 => self.sha256_remote(path).await,
            ChecksumAlgorithm::Md5 => self.md5_remote(path).await,
        }
    }

    pub async fn sha256_remote(&self, path: &str) -> Result<String, AppError> {
        if let Some(hash) = self.try_sha256_remote_via_extension(path).await {
            return Ok(hash);
        }
        if let Some(hash) = self.try_sha256_remote_via_exec(path).await {
            return Ok(hash);
        }
        self.sha256_remote_streaming(path).await
    }

    pub async fn md5_remote(&self, path: &str) -> Result<String, AppError> {
        if let Some(hash) = self.try_md5_remote_via_exec(path).await {
            return Ok(hash);
        }
        self.md5_remote_streaming(path).await
    }

    async fn try_sha256_remote_via_extension(&self, path: &str) -> Option<String> {
        let request = super::check_file::CheckFileRequest::sha256_whole_file(path);
        let _payload = request.encode();
        let _ext = super::check_file::CHECK_FILE_NAME_EXT;
        // 当前底层 russh-sftp 会话封装未直接开放 extended 请求通道，
        // 探测未通过时平滑降级到 SSH 远端命令计算或 SFTP 流式读取。
        None
    }

    async fn try_sha256_remote_via_exec(&self, path: &str) -> Option<String> {
        let remote = self.ssh_remote.as_ref()?;
        let quoted = shell_single_quote(path);
        let command =
            format!("sha256sum -- {quoted} 2>/dev/null || shasum -a 256 -- {quoted} 2>/dev/null");
        match remote.exec_command_capture(&command).await {
            Ok(output) if output.exit_status == Some(0) => {
                if let Some(hash) = parse_hash_output(&output.stdout, 64) {
                    log::debug!(
                        "通过 SSH 远程命令成功计算远端 SHA-256，路径: {path}，哈希: {hash}"
                    );
                    return Some(hash);
                }
                log::debug!(
                    "SSH 远程命令输出未能解析为有效 SHA-256: {}",
                    output.stdout.trim()
                );
                None
            }
            Ok(output) => {
                log::debug!(
                    "SSH 远程哈希命令退出码非 0: {:?}，标准错误: {}",
                    output.exit_status,
                    output.stderr.trim()
                );
                None
            }
            Err(error) => {
                log::debug!("执行 SSH 远程哈希命令失败，将降级为 SFTP 流式读取，原因: {error}");
                None
            }
        }
    }

    async fn try_md5_remote_via_exec(&self, path: &str) -> Option<String> {
        let remote = self.ssh_remote.as_ref()?;
        let quoted = shell_single_quote(path);
        let command = format!("md5sum -- {quoted} 2>/dev/null || md5 -q -- {quoted} 2>/dev/null");
        match remote.exec_command_capture(&command).await {
            Ok(output) if output.exit_status == Some(0) => {
                if let Some(hash) = parse_hash_output(&output.stdout, 32) {
                    log::debug!("通过 SSH 远程命令成功计算远端 MD5，路径: {path}，哈希: {hash}");
                    return Some(hash);
                }
                log::debug!(
                    "SSH 远程命令输出未能解析为有效 MD5: {}",
                    output.stdout.trim()
                );
                None
            }
            Ok(output) => {
                log::debug!(
                    "SSH 远程 MD5 命令退出码非 0: {:?}，标准错误: {}",
                    output.exit_status,
                    output.stderr.trim()
                );
                None
            }
            Err(error) => {
                log::debug!("执行 SSH 远程 MD5 命令失败，将降级为 SFTP 流式读取，原因: {error}");
                None
            }
        }
    }

    pub async fn sha256_remote_streaming(&self, path: &str) -> Result<String, AppError> {
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

    pub async fn md5_remote_streaming(&self, path: &str) -> Result<String, AppError> {
        let mut remote = self.session.open(path).await.map_err(sftp_error)?;
        let mut hasher = Md5::new();
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

    pub async fn quick_sample_remote(&self, path: &str) -> Result<String, AppError> {
        let mut remote = self.session.open(path).await.map_err(sftp_error)?;
        let metadata = remote.metadata().await.map_err(sftp_error)?;
        let file_len = metadata.size.unwrap_or(0);
        let sample_threshold = (SAMPLE_CHUNK * 3) as u64;
        if file_len <= sample_threshold {
            let _ = remote.close().await;
            return self.sha256_remote(path).await;
        }

        let mut hasher = Sha256::new();
        hasher.update(file_len.to_be_bytes());

        let mut buffer = vec![0u8; SAMPLE_CHUNK];
        let count = remote.read(&mut buffer).await.map_err(sftp_io_error)?;
        hasher.update(&buffer[..count]);

        let mid_offset = (file_len / 2).saturating_sub((SAMPLE_CHUNK / 2) as u64);
        remote
            .seek(SeekFrom::Start(mid_offset))
            .await
            .map_err(sftp_io_error)?;
        let count = remote.read(&mut buffer).await.map_err(sftp_io_error)?;
        hasher.update(&buffer[..count]);

        let tail_offset = file_len.saturating_sub(SAMPLE_CHUNK as u64);
        remote
            .seek(SeekFrom::Start(tail_offset))
            .await
            .map_err(sftp_io_error)?;
        let count = remote.read(&mut buffer).await.map_err(sftp_io_error)?;
        hasher.update(&buffer[..count]);

        let _ = remote.close().await;
        Ok(hex_encode(&hasher.finalize()))
    }
}
