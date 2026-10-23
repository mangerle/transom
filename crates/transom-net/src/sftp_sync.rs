//! 目录双向对比与增量同步的 SFTP 客户端实现。

use crate::sftp::SftpClient;
use crate::sftp_dir::{join_remote_path, local_io_error, local_path_for, scan_local_snapshot};
use std::path::Path;
use transom_core::{
    AppError, DirComparison, SyncDirection, SyncItem, TransferFilter, TransferSummary,
    build_dir_comparison,
};

impl SftpClient {
    /// 对比本地与远端目录，返回需要同步的差异项。
    ///
    /// # Errors
    /// 排除规则非法、任一侧目录不可读或远端会话中断时返回
    /// `ErrorCode::Configuration` 或 `ErrorCode::SftpFailed`。
    pub async fn compare_directories(
        &self,
        local_root: &Path,
        remote_root: &str,
        filter: &TransferFilter,
    ) -> Result<DirComparison, AppError> {
        let matcher = filter.matcher()?;
        let local = scan_local_snapshot(local_root, &matcher).await?;
        let remote = self.scan_remote_snapshot(remote_root, &matcher).await?;
        Ok(build_dir_comparison(
            local.entries,
            remote.entries,
            local.skipped.saturating_add(remote.skipped),
        ))
    }

    /// 按差异计划执行增量同步，目录先创建、文件后传输。
    ///
    /// 同步使用覆盖模式而非断点续传：差异项两端内容不同，续传可能把旧内容
    /// 追加到新内容之后，覆盖写入才能保证同步结果与源端一致。
    ///
    /// # Errors
    /// 任一目录创建或文件传输失败时返回 `ErrorCode::SftpFailed`；已完成的
    /// 项目会保留在目标端，用户可通过重试继续剩余项。
    pub async fn sync_items<F>(
        &self,
        local_root: &Path,
        remote_root: &str,
        items: &[SyncItem],
        mut progress: F,
    ) -> Result<TransferSummary, AppError>
    where
        F: FnMut(u64, Option<u64>) + Send,
    {
        let total = items.iter().fold(0u64, |total, item| {
            if item.is_dir {
                total
            } else {
                total.saturating_add(item.size)
            }
        });
        progress(0, Some(total));

        let mut ordered = items.iter().collect::<Vec<_>>();
        ordered.sort_by(|left, right| {
            right
                .is_dir
                .cmp(&left.is_dir)
                .then_with(|| {
                    left.relative_path
                        .matches('/')
                        .count()
                        .cmp(&right.relative_path.matches('/').count())
                })
                .then_with(|| left.relative_path.cmp(&right.relative_path))
        });

        let mut completed = 0u64;
        for item in ordered {
            match (item.direction, item.is_dir) {
                (SyncDirection::Upload, true) => {
                    let remote_path = join_remote_path(remote_root, &item.relative_path);
                    self.create_dir_all(&remote_path).await?;
                }
                (SyncDirection::Download, true) => {
                    let local_path = local_path_for(local_root, &item.relative_path)?;
                    tokio::fs::create_dir_all(&local_path)
                        .await
                        .map_err(|error| local_io_error("创建本地目录失败", &local_path, error))?;
                }
                (SyncDirection::Upload, false) => {
                    let local_path = local_path_for(local_root, &item.relative_path)?;
                    let remote_path = join_remote_path(remote_root, &item.relative_path);
                    if let Some(parent) = remote_parent(&remote_path) {
                        self.create_dir_all(parent).await?;
                    }
                    let base = completed;
                    let transferred = self
                        .upload_resumable(&local_path, &remote_path, false, |transferred, _| {
                            progress(base.saturating_add(transferred), Some(total));
                        })
                        .await?;
                    completed = completed.saturating_add(transferred);
                }
                (SyncDirection::Download, false) => {
                    let local_path = local_path_for(local_root, &item.relative_path)?;
                    let remote_path = join_remote_path(remote_root, &item.relative_path);
                    if let Some(parent) = local_path.parent() {
                        tokio::fs::create_dir_all(parent)
                            .await
                            .map_err(|error| local_io_error("创建本地父目录失败", parent, error))?;
                    }
                    let base = completed;
                    let transferred = self
                        .download_resumable(&remote_path, &local_path, false, |transferred, _| {
                            progress(base.saturating_add(transferred), Some(total));
                        })
                        .await?;
                    completed = completed.saturating_add(transferred);
                }
            }
        }

        Ok(TransferSummary {
            transferred: completed,
            skipped: 0,
        })
    }
}

/// 取远端路径的父目录；根目录或单层相对路径返回 `None`/根。
fn remote_parent(path: &str) -> Option<&str> {
    let trimmed = path.trim_end_matches('/');
    let (parent, _) = trimmed.rsplit_once('/')?;
    if parent.is_empty() {
        Some("/")
    } else {
        Some(parent)
    }
}

#[cfg(test)]
mod tests {
    use super::remote_parent;

    #[test]
    fn remote_parent_handles_absolute_and_relative_paths() {
        assert_eq!(remote_parent("/srv/app/file.txt"), Some("/srv/app"));
        assert_eq!(remote_parent("/file.txt"), Some("/"));
        assert_eq!(remote_parent("app/file.txt"), Some("app"));
        assert_eq!(remote_parent("file.txt"), None);
    }
}
