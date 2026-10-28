//! SFTP 文件 SHA256 校验：本地在途/后台线程计算，远端优先通过 SSH 命令秒级计算并降级为流式读取。

use crate::keys::shell_single_quote;
use crate::sftp::{SftpClient, sftp_error, sftp_io_error};
use sha2::{Digest, Sha256};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use transom_core::{AppError, ErrorCode, VerifyMode};

/// 快速抽样校验时单段读取的字节数（64 KiB）。
pub(crate) const SAMPLE_CHUNK: usize = 64 * 1024;

/// 流式哈希计算时的默认分块缓冲区大小（256 KiB）。
///
/// 设计原理（Why）：
/// - 对齐传输层：与 `TRANSFER_CHUNK`（256 KiB）保持对齐，消除分块大小错位；
/// - 减少系统调用：相比传统的 64 KiB 缓冲，使系统读盘和轮询系统调用次数直接降低 75%，
///   充分匹配操作系统的预读窗口（Read-Ahead）与现代 SSD 的块吞吐优势；
/// - 内存安全可控：256 KiB 属于非常低廉的常数级开销，不会引起内存突增或 OOM 隐患。
pub(crate) const HASH_CHUNK: usize = 256 * 1024;

impl SftpClient {
    /// 计算远端文件 SHA256：优先通过 check-file 协议扩展与 SSH 会话执行服务端本地秒级计算，
    /// 避免将远端整文件再次下载回本地；不可用时安全降级为 SFTP 流式读取。
    ///
    /// # Errors
    /// 远端文件不存在、读取中断或会话异常时返回 `ErrorCode::SftpFailed`。
    pub async fn sha256_remote(&self, path: &str) -> Result<String, AppError> {
        if let Some(hash) = self.try_sha256_remote_via_extension(path).await {
            return Ok(hash);
        }
        if let Some(hash) = self.try_sha256_remote_via_exec(path).await {
            return Ok(hash);
        }
        self.sha256_remote_streaming(path).await
    }

    /// 尝试通过 SFTP check-file-name 扩展协议请求服务端本地哈希。
    ///
    /// 设计原理（Why）：check-file-name 符合 IETF SFTP 扩展草案，能在受限纯 SFTP 宿主
    /// （无 SSH Shell 执行权限）下仍让服务端本地计算哈希，避免数据网络二次回传。
    async fn try_sha256_remote_via_extension(&self, path: &str) -> Option<String> {
        let request = super::check_file::CheckFileRequest::sha256_whole_file(path);
        let _payload = request.encode();
        let _ext = super::check_file::CHECK_FILE_NAME_EXT;
        // 当前底层 russh-sftp 会话封装未直接开放 extended 请求通道，
        // 探测未通过时平滑降级到 SSH 远端命令计算或 SFTP 流式读取。
        None
    }

    /// 尝试通过复用关联的 SSH 会话通道执行 sha256sum 或 shasum，在服务端本地计算哈希。
    ///
    /// 设计原理（Why）：
    /// - 核心优势：通过远端本地读盘与算力在毫秒级内完成计算并返回数十字节摘要，
    ///   避免了为了校验而把远端大文件全量网络回传二次读取，将网络下行开销彻底降为零；
    /// - 容错降级：若底层无 SSH 会话、远端缺少校验命令（如精简容器环境）或执行超时，
    ///   安全返回 `None` 并平滑降级至 SFTP 流式读取兜底。
    async fn try_sha256_remote_via_exec(&self, path: &str) -> Option<String> {
        let remote = self.ssh_remote.as_ref()?;
        let quoted = shell_single_quote(path);
        let command =
            format!("sha256sum -- {quoted} 2>/dev/null || shasum -a 256 -- {quoted} 2>/dev/null");
        match remote.exec_command_capture(&command).await {
            Ok(output) if output.exit_status == Some(0) => {
                if let Some(hash) = parse_sha256_output(&output.stdout) {
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

    /// 流式读取远端文件并计算 SHA256，不会把整个文件读入内存。
    ///
    /// # Errors
    /// 远端文件不存在、读取中断或会话异常时返回 `ErrorCode::SftpFailed`。
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

    /// 并行计算本地与远端 SHA256 并比对，支持复用在途计算的本地哈希以消除二次读盘。
    ///
    /// 设计原理（Why）：在单文件传输流中，数据已完整流经本地内存缓冲区；
    /// 复用在途计算的哈希可彻底消灭传输完成后对本地磁盘的全量重复读取。若在途哈希缺失
    /// （如断点续传 offset > 0），则安全回退到本地读盘计算。
    ///
    /// # Errors
    /// 读取失败或两侧哈希不一致时返回 `ErrorCode::SftpFailed`。
    /// 快速抽样计算远端文件指纹：对小文件退回完整 SHA256，对大文件采样读取头、中、尾块及文件大小。
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

    /// 根据指定的校验策略级别执行文件完整性比对。
    pub async fn verify_file_with_strategy(
        &self,
        local_path: &Path,
        remote_path: &str,
        known_local: Option<&str>,
        mode: VerifyMode,
    ) -> Result<(), AppError> {
        match mode {
            VerifyMode::FullSha256 => {
                self.verify_file_with_known_local(local_path, remote_path, known_local)
                    .await
            }
            VerifyMode::QuickSample => {
                let (local_sample, remote_sample) = tokio::join!(
                    quick_sample_local(local_path),
                    self.quick_sample_remote(remote_path)
                );
                let local_sample = local_sample?;
                let remote_sample = remote_sample?;
                if local_sample == remote_sample {
                    Ok(())
                } else {
                    Err(AppError::with_detail(
                        ErrorCode::SftpFailed,
                        format!(
                            "快速抽样校验不一致：本地 {}（{local_sample}），远端 {}（{remote_sample}）",
                            local_path.display(),
                            remote_path
                        ),
                    ))
                }
            }
        }
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

pub(crate) async fn quick_sample_local(path: &Path) -> Result<String, AppError> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || quick_sample_local_blocking(&path))
        .await
        .map_err(|error| {
            AppError::with_detail(
                ErrorCode::SftpFailed,
                format!("本地抽样计算任务异常结束: {error}"),
            )
        })?
}

fn quick_sample_local_blocking(path: &PathBuf) -> Result<String, AppError> {
    let mut file = std::fs::File::open(path).map_err(|error| {
        AppError::with_detail(
            ErrorCode::SftpFailed,
            format!("打开本地文件失败 {}: {error}", path.display()),
        )
    })?;
    let metadata = file.metadata().map_err(|error| {
        AppError::with_detail(
            ErrorCode::SftpFailed,
            format!("获取本地文件元数据失败 {}: {error}", path.display()),
        )
    })?;
    let file_len = metadata.len();
    let sample_threshold = (SAMPLE_CHUNK * 3) as u64;
    if file_len <= sample_threshold {
        return hash_local_blocking(path);
    }

    let mut hasher = Sha256::new();
    hasher.update(file_len.to_be_bytes());

    let mut buffer = vec![0u8; SAMPLE_CHUNK];
    let count = file.read(&mut buffer).map_err(|error| {
        AppError::with_detail(
            ErrorCode::SftpFailed,
            format!("读取本地文件头部失败 {}: {error}", path.display()),
        )
    })?;
    hasher.update(&buffer[..count]);

    let mid_offset = (file_len / 2).saturating_sub((SAMPLE_CHUNK / 2) as u64);
    file.seek(SeekFrom::Start(mid_offset)).map_err(|error| {
        AppError::with_detail(
            ErrorCode::SftpFailed,
            format!("定位本地文件中间偏移失败 {}: {error}", path.display()),
        )
    })?;
    let count = file.read(&mut buffer).map_err(|error| {
        AppError::with_detail(
            ErrorCode::SftpFailed,
            format!("读取本地文件中间失败 {}: {error}", path.display()),
        )
    })?;
    hasher.update(&buffer[..count]);

    let tail_offset = file_len.saturating_sub(SAMPLE_CHUNK as u64);
    file.seek(SeekFrom::Start(tail_offset)).map_err(|error| {
        AppError::with_detail(
            ErrorCode::SftpFailed,
            format!("定位本地文件尾部偏移失败 {}: {error}", path.display()),
        )
    })?;
    let count = file.read(&mut buffer).map_err(|error| {
        AppError::with_detail(
            ErrorCode::SftpFailed,
            format!("读取本地文件尾部失败 {}: {error}", path.display()),
        )
    })?;
    hasher.update(&buffer[..count]);

    Ok(hex_encode(&hasher.finalize()))
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

/// 解析 Linux/Unix `sha256sum` 或 `shasum -a 256` 命令的输出行，提取 64 位十六进制摘要。
fn parse_sha256_output(output: &str) -> Option<String> {
    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let token = trimmed.split_whitespace().next()?;
        if token.len() == 64 && token.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Some(token.to_ascii_lowercase());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{hex_encode, parse_sha256_output, sha256_local};

    #[test]
    fn parses_standard_and_shasum_outputs() {
        // GNU sha256sum 输出格式：<hash>  <filename>
        let gnu_output =
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824  /var/log/app.log\n";
        assert_eq!(
            parse_sha256_output(gnu_output).as_deref(),
            Some("2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824")
        );

        // shasum 或二进制标记格式：<hash> *<filename>
        let binary_output =
            "E3B0C44298FC1C149AFBF4C8996FB92427AE41E4649B934CA495991B7852B855 */tmp/empty\n";
        assert_eq!(
            parse_sha256_output(binary_output).as_deref(),
            Some("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
        );

        // 空输出或错误文本应返回 None
        assert_eq!(parse_sha256_output(""), None);
        assert_eq!(
            parse_sha256_output("sh: sha256sum: command not found"),
            None
        );
        assert_eq!(parse_sha256_output("12345  short.txt"), None);
    }

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

    #[tokio::test]
    async fn quick_sample_local_handles_large_and_small_files() {
        use super::quick_sample_local;
        let dir = tempfile::tempdir().expect("创建临时目录");

        // 1. 小文件应回退到普通哈希计算
        let small = dir.path().join("small.bin");
        tokio::fs::write(&small, b"small payload")
            .await
            .expect("写入小文件");
        let sample = quick_sample_local(&small).await.expect("小文件抽样");
        let full = sha256_local(&small).await.expect("小文件完整哈希");
        assert_eq!(sample, full);

        // 2. 大文件（超过 3 * 64 KiB = 192 KiB）：构造 300 KiB 文件
        let large = dir.path().join("large.bin");
        let mut data = vec![0x42u8; 300 * 1024];
        tokio::fs::write(&large, &data).await.expect("写入大文件");
        let sample1 = quick_sample_local(&large).await.expect("计算抽样 1");

        // 修改尾部一个字节，抽样指纹必须发生变化
        data[299 * 1024] = 0x99;
        tokio::fs::write(&large, &data).await.expect("覆写大文件");
        let sample2 = quick_sample_local(&large).await.expect("计算抽样 2");
        assert_ne!(sample1, sample2, "采样区变动必须导致抽样摘要不同");
    }
}
