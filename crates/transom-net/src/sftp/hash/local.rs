//! 本地文件哈希与快速抽样。

use super::{HASH_CHUNK, SAMPLE_CHUNK};
use md5::Md5;
use sha2::{Digest, Sha256};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use transom_core::{AppError, ChecksumAlgorithm, ErrorCode};

/// 根据算法计算本地文件的完整摘要。
pub(crate) async fn checksum_local(
    path: &Path,
    algorithm: ChecksumAlgorithm,
) -> Result<String, AppError> {
    match algorithm {
        ChecksumAlgorithm::Sha256 => sha256_local(path).await,
        ChecksumAlgorithm::Md5 => md5_local(path).await,
    }
}

pub(crate) async fn sha256_local(path: &Path) -> Result<String, AppError> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || hash_local_blocking::<Sha256>(&path, "SHA256"))
        .await
        .map_err(|error| {
            AppError::with_detail(
                ErrorCode::SftpFailed,
                format!("本地 SHA256 计算任务异常结束: {error}"),
            )
        })?
}

pub(crate) async fn md5_local(path: &Path) -> Result<String, AppError> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || hash_local_blocking::<Md5>(&path, "MD5"))
        .await
        .map_err(|error| {
            AppError::with_detail(
                ErrorCode::SftpFailed,
                format!("本地 MD5 计算任务异常结束: {error}"),
            )
        })?
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
        return hash_local_blocking::<Sha256>(path, "SHA256");
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

fn hash_local_blocking<D: Digest + Default>(
    path: &PathBuf,
    algo_name: &str,
) -> Result<String, AppError> {
    let mut file = std::fs::File::open(path).map_err(|error| {
        AppError::with_detail(
            ErrorCode::SftpFailed,
            format!("打开本地文件失败 {}: {error}", path.display()),
        )
    })?;
    let mut hasher = D::new();
    let mut buffer = vec![0u8; HASH_CHUNK];
    loop {
        let count = file.read(&mut buffer).map_err(|error| {
            AppError::with_detail(
                ErrorCode::SftpFailed,
                format!(
                    "读取本地文件失败进行 {algo_name} 计算 {}: {error}",
                    path.display()
                ),
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

/// 解析 Linux/Unix 校验命令（sha256sum、md5sum 等）的输出行，提取指定长度的十六进制摘要。
pub(super) fn parse_hash_output(output: &str, expected_len: usize) -> Option<String> {
    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let token = trimmed.split_whitespace().next()?;
        if token.len() == expected_len && token.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Some(token.to_ascii_lowercase());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_standard_and_shasum_outputs() {
        // GNU sha256sum 输出格式：<hash>  <filename>
        let gnu_output =
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824  /var/log/app.log\n";
        assert_eq!(
            parse_hash_output(gnu_output, 64).as_deref(),
            Some("2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824")
        );

        // shasum 或二进制标记格式：<hash> *<filename>
        let binary_output =
            "E3B0C44298FC1C149AFBF4C8996FB92427AE41E4649B934CA495991B7852B855 */tmp/empty\n";
        assert_eq!(
            parse_hash_output(binary_output, 64).as_deref(),
            Some("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
        );

        // md5sum 输出格式：<hash>  <filename>
        let md5_output = "5d41402abc4b2a76b9719d911017c592  hello.txt\n";
        assert_eq!(
            parse_hash_output(md5_output, 32).as_deref(),
            Some("5d41402abc4b2a76b9719d911017c592")
        );

        // 空输出或错误文本应返回 None
        assert_eq!(parse_hash_output("", 64), None);
        assert_eq!(
            parse_hash_output("sh: sha256sum: command not found", 64),
            None
        );
        assert_eq!(parse_hash_output("12345  short.txt", 64), None);
    }

    #[test]
    fn hex_encode_uses_lowercase_and_pads_bytes() {
        assert_eq!(hex_encode(&[0x00, 0x0f, 0xff]), "000fff");
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

    #[tokio::test]
    async fn local_md5_matches_known_digests() {
        let dir = tempfile::tempdir().expect("创建临时目录");
        let hello = dir.path().join("hello.txt");
        tokio::fs::write(&hello, b"hello")
            .await
            .expect("写入测试文件");
        assert_eq!(
            md5_local(&hello).await.expect("计算 hello MD5"),
            "5d41402abc4b2a76b9719d911017c592"
        );
        assert_eq!(
            checksum_local(&hello, ChecksumAlgorithm::Md5)
                .await
                .expect("动态分发 MD5"),
            "5d41402abc4b2a76b9719d911017c592"
        );

        let empty = dir.path().join("empty.txt");
        tokio::fs::write(&empty, b"").await.expect("写入空文件");
        assert_eq!(
            md5_local(&empty).await.expect("计算空文件 MD5"),
            "d41d8cd98f00b204e9800998ecf8427e"
        );
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
