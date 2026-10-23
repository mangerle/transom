//! SFTP 目录递归传输：本地/远端目录树扫描、排除规则剪枝与逐文件传输。

use crate::sftp::{SftpClient, sftp_error};
use std::path::{Path, PathBuf};
use transom_core::{
    AppError, DirEntryMeta, DirSnapshotEntry, ErrorCode, TransferFilter, TransferMatcher,
    TransferSummary,
};

/// 单侧目录扫描结果。
pub(crate) struct ScanResult {
    pub entries: Vec<DirSnapshotEntry>,
    pub skipped: u64,
}

impl ScanResult {
    pub(crate) fn total_bytes(&self) -> u64 {
        self.entries.iter().fold(0u64, |total, entry| {
            if entry.meta.is_dir {
                total
            } else {
                total.saturating_add(entry.meta.size)
            }
        })
    }
}

impl SftpClient {
    /// 递归上传本地目录，返回整体字节数与跳过条目数。
    ///
    /// 设计原理（Why）：
    /// - 实现初衷：目录传输需要先建立完整文件清单才能给出准确总进度，因此在
    ///   开始传输前完成一次只读扫描；
    /// - 核心优势：排除规则在扫描阶段对目录直接剪枝，`.git` 等大目录不会被
    ///   逐文件遍历；扫描结果同时用于预创建空目录，保证空目录也能同步；
    /// - 代价与局限：扫描与传输之间如果文件被外部修改，总进度可能略有偏差，
    ///   因此进度计算全部使用饱和算术，不追求严格字节守恒。
    ///
    /// # Errors
    /// 排除规则非法、本地目录不可读、远端建目录或文件上传失败时返回
    /// `ErrorCode::Configuration` 或 `ErrorCode::SftpFailed`。符号链接会被跳过。
    pub async fn upload_dir<F>(
        &self,
        local_root: &Path,
        remote_root: &str,
        resume: bool,
        filter: &TransferFilter,
        verify: bool,
        mut progress: F,
    ) -> Result<TransferSummary, AppError>
    where
        F: FnMut(u64, Option<u64>) + Send,
    {
        let matcher = filter.matcher()?;
        let scan = scan_local_snapshot(local_root, &matcher).await?;
        let total = scan.total_bytes();
        progress(0, Some(total));

        self.create_dir_all(remote_root).await?;
        for relative in sorted_directories(&scan.entries) {
            let remote_path = join_remote_path(remote_root, relative);
            self.ensure_dir(&remote_path).await?;
        }

        let mut completed = 0u64;
        for file in sorted_files(&scan.entries) {
            let local_path = local_path_for(local_root, &file.relative_path)?;
            let remote_path = join_remote_path(remote_root, &file.relative_path);
            let base = completed;
            let transferred = self
                .upload_resumable(&local_path, &remote_path, resume, |transferred, _| {
                    progress(base.saturating_add(transferred), Some(total));
                })
                .await?;
            if verify {
                self.verify_file(&local_path, &remote_path).await?;
            }
            completed = completed.saturating_add(transferred);
        }

        Ok(TransferSummary {
            transferred: completed,
            skipped: scan.skipped,
            verified: verify,
        })
    }

    /// 递归下载远端目录，返回整体字节数与跳过条目数。
    ///
    /// 安全约束：远端返回的文件名会参与本地路径拼接，任何空片段、`.`、`..`
    /// 或路径分隔符都会被拒绝，防止恶意服务端把文件写到目标目录之外。
    ///
    /// # Errors
    /// 排除规则非法、远端目录不可读、本地建目录或文件写入失败时返回
    /// `ErrorCode::Configuration` 或 `ErrorCode::SftpFailed`。符号链接会被跳过。
    pub async fn download_dir<F>(
        &self,
        remote_root: &str,
        local_root: &Path,
        resume: bool,
        filter: &TransferFilter,
        verify: bool,
        mut progress: F,
    ) -> Result<TransferSummary, AppError>
    where
        F: FnMut(u64, Option<u64>) + Send,
    {
        let matcher = filter.matcher()?;
        let scan = self.scan_remote_snapshot(remote_root, &matcher).await?;
        let total = scan.total_bytes();
        progress(0, Some(total));

        tokio::fs::create_dir_all(local_root)
            .await
            .map_err(|error| local_io_error("创建本地目录失败", local_root, error))?;
        for relative in sorted_directories(&scan.entries) {
            let local_path = local_path_for(local_root, relative)?;
            tokio::fs::create_dir_all(&local_path)
                .await
                .map_err(|error| local_io_error("创建本地目录失败", &local_path, error))?;
        }

        let mut completed = 0u64;
        for file in sorted_files(&scan.entries) {
            let remote_path = join_remote_path(remote_root, &file.relative_path);
            let local_path = local_path_for(local_root, &file.relative_path)?;
            if let Some(parent) = local_path.parent() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .map_err(|error| local_io_error("创建本地父目录失败", parent, error))?;
            }
            let base = completed;
            let transferred = self
                .download_resumable(&remote_path, &local_path, resume, |transferred, _| {
                    progress(base.saturating_add(transferred), Some(total));
                })
                .await?;
            if verify {
                self.verify_file(&local_path, &remote_path).await?;
            }
            completed = completed.saturating_add(transferred);
        }

        Ok(TransferSummary {
            transferred: completed,
            skipped: scan.skipped,
            verified: verify,
        })
    }

    /// 递归扫描远端目录，供目录对比与同步使用。
    pub(crate) async fn scan_remote_snapshot(
        &self,
        root: &str,
        matcher: &TransferMatcher,
    ) -> Result<ScanResult, AppError> {
        let canonical = self.session.canonicalize(root).await.map_err(sftp_error)?;
        let mut entries = Vec::new();
        let mut skipped = 0u64;
        let mut stack: Vec<(String, String)> = vec![(canonical, String::new())];
        while let Some((directory, relative_prefix)) = stack.pop() {
            let children = self
                .session
                .read_dir(&directory)
                .await
                .map_err(sftp_error)?;
            let mut batch = Vec::new();
            for entry in children {
                let name = entry.file_name();
                let metadata = entry.metadata();
                batch.push((
                    name,
                    metadata.is_dir(),
                    metadata.is_symlink(),
                    metadata.size.unwrap_or(0),
                    metadata.mtime,
                ));
            }
            batch.sort_by(|left, right| left.0.cmp(&right.0));

            for (name, is_dir, is_symlink, size, mtime) in batch {
                if !safe_component(&name) || is_symlink {
                    skipped = skipped.saturating_add(1);
                    continue;
                }
                let relative = join_relative(&relative_prefix, &name);
                if matcher.matches(&relative, is_dir) {
                    skipped = skipped.saturating_add(1);
                    continue;
                }
                entries.push(DirSnapshotEntry {
                    relative_path: relative.clone(),
                    meta: DirEntryMeta {
                        is_dir,
                        size,
                        mtime,
                    },
                });
                if is_dir {
                    stack.push((join_remote_path(&directory, &name), relative));
                }
            }
        }
        entries.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
        Ok(ScanResult { entries, skipped })
    }
}

/// 递归扫描本地目录，供目录传输、对比与同步使用。
pub(crate) async fn scan_local_snapshot(
    root: &Path,
    matcher: &TransferMatcher,
) -> Result<ScanResult, AppError> {
    let metadata = tokio::fs::symlink_metadata(root)
        .await
        .map_err(|error| local_io_error("读取本地目录失败", root, error))?;
    if !metadata.is_dir() {
        return Err(AppError::with_detail(
            ErrorCode::SftpFailed,
            format!("本地路径不是目录: {}", root.display()),
        ));
    }

    let mut entries = Vec::new();
    let mut skipped = 0u64;
    let mut stack: Vec<(PathBuf, String)> = vec![(root.to_path_buf(), String::new())];
    while let Some((directory, relative_prefix)) = stack.pop() {
        let mut read_dir = tokio::fs::read_dir(&directory)
            .await
            .map_err(|error| local_io_error("读取本地目录失败", &directory, error))?;
        let mut children = Vec::new();
        while let Some(entry) = read_dir
            .next_entry()
            .await
            .map_err(|error| local_io_error("读取本地目录项失败", &directory, error))?
        {
            children.push(entry);
        }
        children.sort_by_key(|entry| entry.file_name());

        for entry in children {
            let name = entry.file_name().to_string_lossy().to_string();
            if !safe_component(&name) {
                skipped = skipped.saturating_add(1);
                continue;
            }
            let file_type = match entry.file_type().await {
                Ok(file_type) => file_type,
                Err(_) => {
                    skipped = skipped.saturating_add(1);
                    continue;
                }
            };
            if file_type.is_symlink() {
                skipped = skipped.saturating_add(1);
                continue;
            }
            let relative = join_relative(&relative_prefix, &name);
            let is_dir = file_type.is_dir();
            if matcher.matches(&relative, is_dir) {
                skipped = skipped.saturating_add(1);
                continue;
            }
            let metadata = entry.metadata().await.ok();
            let size = metadata.as_ref().map(|meta| meta.len()).unwrap_or(0);
            let mtime = unix_seconds(metadata.and_then(|meta| meta.modified().ok()));
            entries.push(DirSnapshotEntry {
                relative_path: relative.clone(),
                meta: DirEntryMeta {
                    is_dir,
                    size,
                    mtime,
                },
            });
            if is_dir {
                stack.push((entry.path(), relative));
            }
        }
    }
    entries.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(ScanResult { entries, skipped })
}

/// 目录按层级深度排序，保证父目录先于子目录创建。
pub(crate) fn sorted_directories(entries: &[DirSnapshotEntry]) -> Vec<&str> {
    let mut directories = entries
        .iter()
        .filter(|entry| entry.meta.is_dir)
        .map(|entry| entry.relative_path.as_str())
        .collect::<Vec<_>>();
    directories.sort_by(|left, right| {
        left.matches('/')
            .count()
            .cmp(&right.matches('/').count())
            .then_with(|| left.cmp(right))
    });
    directories
}

pub(crate) fn sorted_files(entries: &[DirSnapshotEntry]) -> Vec<&DirSnapshotEntry> {
    let mut files = entries
        .iter()
        .filter(|entry| !entry.meta.is_dir)
        .collect::<Vec<_>>();
    files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    files
}

fn join_relative(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}/{name}")
    }
}

pub(crate) fn join_remote_path(root: &str, relative: &str) -> String {
    if root == "/" {
        format!("/{relative}")
    } else {
        format!("{}/{}", root.trim_end_matches('/'), relative)
    }
}

/// 将相对路径逐段拼接到本地根目录，并拒绝任何越界片段。
pub(crate) fn local_path_for(root: &Path, relative: &str) -> Result<PathBuf, AppError> {
    let mut path = root.to_path_buf();
    for component in relative.split('/') {
        if !safe_component(component) {
            return Err(AppError::with_detail(
                ErrorCode::SftpFailed,
                format!("传输路径包含非法片段: {relative}"),
            ));
        }
        path.push(component);
    }
    Ok(path)
}

pub(crate) fn safe_component(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains('/') && !name.contains('\\')
}

pub(crate) fn local_io_error(context: &str, path: &Path, error: std::io::Error) -> AppError {
    AppError::with_detail(
        ErrorCode::SftpFailed,
        format!("{context} {}: {error}", path.display()),
    )
}

fn unix_seconds(time: Option<std::time::SystemTime>) -> Option<u32> {
    let seconds = time?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    Some(seconds.min(u64::from(u32::MAX)) as u32)
}

#[cfg(test)]
mod tests {
    use super::{join_remote_path, local_path_for, safe_component};
    use std::path::Path;

    #[test]
    fn rejects_path_traversal_components() {
        assert!(local_path_for(Path::new("/tmp/root"), "../escape").is_err());
        assert!(local_path_for(Path::new("/tmp/root"), "a/../b").is_err());
        assert!(local_path_for(Path::new("/tmp/root"), "a\\b").is_err());
        assert!(local_path_for(Path::new("/tmp/root"), "").is_err());
        assert_eq!(
            local_path_for(Path::new("/tmp/root"), "a/b.txt").expect("合法路径"),
            Path::new("/tmp/root/a/b.txt")
        );
    }

    #[test]
    fn component_validation_and_remote_join() {
        assert!(safe_component("normal.txt"));
        assert!(!safe_component(".."));
        assert!(!safe_component("a/b"));
        assert_eq!(join_remote_path("/", "a/b"), "/a/b");
        assert_eq!(join_remote_path("/srv/app/", "a/b"), "/srv/app/a/b");
        assert_eq!(join_remote_path("relative", "a"), "relative/a");
    }
}
