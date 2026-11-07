//! 传输校验策略：按 VerifyOptions 选择算法与已知摘要比对。

use super::local::{checksum_local, quick_sample_local};
use crate::sftp::{SftpClient, TransferProgressResult};
use std::path::Path;
use transom_core::{AppError, ChecksumAlgorithm, ErrorCode, VerifyMode, VerifyOptions};

impl SftpClient {
    pub async fn verify_transfer(
        &self,
        local_path: &Path,
        remote_path: &str,
        result: &TransferProgressResult,
        options: VerifyOptions,
    ) -> Result<(), AppError> {
        if !options.enabled {
            return Ok(());
        }
        let known_local = if options.algorithm == ChecksumAlgorithm::Sha256 {
            result.local_sha256.as_deref()
        } else {
            None
        };
        self.verify_file_with_strategy_and_algo(
            local_path,
            remote_path,
            known_local,
            options.mode,
            options.algorithm,
        )
        .await
    }

    pub async fn verify_file_with_strategy(
        &self,
        local_path: &Path,
        remote_path: &str,
        known_local: Option<&str>,
        mode: VerifyMode,
    ) -> Result<(), AppError> {
        self.verify_file_with_strategy_and_algo(
            local_path,
            remote_path,
            known_local,
            mode,
            ChecksumAlgorithm::Sha256,
        )
        .await
    }

    pub async fn verify_file_with_strategy_and_algo(
        &self,
        local_path: &Path,
        remote_path: &str,
        known_local: Option<&str>,
        mode: VerifyMode,
        algorithm: ChecksumAlgorithm,
    ) -> Result<(), AppError> {
        match mode {
            VerifyMode::FullSha256 => {
                self.verify_file_with_algorithm(local_path, remote_path, known_local, algorithm)
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
                        ErrorCode::SftpVerifyFailed,
                        format!(
                            "快速抽样指纹不一致：本地 {}（{local_sample}），远端 {}（{remote_sample}）",
                            local_path.display(),
                            remote_path
                        ),
                    ))
                }
            }
        }
    }

    pub async fn verify_file_with_known_local(
        &self,
        local_path: &Path,
        remote_path: &str,
        known_local: Option<&str>,
    ) -> Result<(), AppError> {
        self.verify_file_with_algorithm(
            local_path,
            remote_path,
            known_local,
            ChecksumAlgorithm::Sha256,
        )
        .await
    }

    pub async fn verify_file_with_algorithm(
        &self,
        local_path: &Path,
        remote_path: &str,
        known_local: Option<&str>,
        algorithm: ChecksumAlgorithm,
    ) -> Result<(), AppError> {
        let (local_meta, remote_meta) = tokio::join!(
            tokio::fs::metadata(local_path),
            self.session.metadata(remote_path)
        );
        if let (Ok(lm), Ok(rm)) = (local_meta, remote_meta)
            && let (local_len, Some(remote_len)) = (lm.len(), rm.size)
            && local_len != remote_len
        {
            return Err(AppError::with_detail(
                ErrorCode::SftpVerifyFailed,
                format!(
                    "两端文件大小不一致：本地 {}（{local_len} 字节），远端 {}（{remote_len} 字节）",
                    local_path.display(),
                    remote_path
                ),
            ));
        }

        let (local_hash, remote_hash) = match known_local {
            Some(known) => (
                Ok(known.to_string()),
                self.checksum_remote(remote_path, algorithm).await,
            ),
            None => tokio::join!(
                checksum_local(local_path, algorithm),
                self.checksum_remote(remote_path, algorithm)
            ),
        };
        let local_hash = local_hash?;
        let remote_hash = remote_hash?;
        if local_hash == remote_hash {
            Ok(())
        } else {
            let label = algorithm.label();
            Err(AppError::with_detail(
                ErrorCode::SftpVerifyFailed,
                format!(
                    "{label} 校验不一致：本地 {}（{local_hash}），远端 {}（{remote_hash}）",
                    local_path.display(),
                    remote_path
                ),
            ))
        }
    }

    pub async fn verify_file(&self, local_path: &Path, remote_path: &str) -> Result<(), AppError> {
        self.verify_file_with_known_local(local_path, remote_path, None)
            .await
    }
}
