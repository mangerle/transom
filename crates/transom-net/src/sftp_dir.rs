//! SFTP 目录递归传输：本地/远端目录树扫描、排除规则剪枝与逐文件传输。

use crate::sftp::{SftpClient, sftp_error};
use std::path::{Path, PathBuf};
use transom_core::{AppError, ErrorCode, TransferFilter, TransferMatcher, TransferSummary};

#[derive(Debug, Clone)]
struct ScannedFile {
    relative: String,
    size: u64,
}

#[derive(Debug, Default)]
struct ScannedTree {
    directories: Vec<String>,
    files: Vec<ScannedFile>,
    skipped: u64,
}

impl ScannedTree {
    fn total_bytes(&self) -> u64 {
        self.files
            .iter()
            .fold(0u64, |total, file| total.saturating_add(file.size))
    }

    fn sort(&mut self) {
        self.directories.sort_by(|left, right| {
            left.matches('/')
                .count()
                .cmp(&right.matches('/').count())
                .then_with(|| left.cmp(right))
        });
        self.directories.dedup();
        self.files
            .sort_by(|left, right| left.relative.cmp(&right.relative));
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
        mut progress: F,
    ) -> Result<TransferSummary, AppError>
    where
        F: FnMut(u64, Option<u64>) + Send,
    {
        let matcher = filter.matcher()?;
        let tree = scan_local_tree(local_root, &matcher).await?;
        let total = tree.total_bytes();
        progress(0, Some(total));

        self.create_dir_all(remote_root).await?;
        for relative in &tree.directories {
            let remote_path = join_remote_path(remote_root, relative);
            self.ensure_dir(&remote_path).await?;
        }

        let mut completed = 0u64;
        for file in &tree.files {
            let local_path = local_path_for(local_root, &file.relative)?;
            let remote_path = join_remote_path(remote_root, &file.relative);
            let base = completed;
            let transferred = self
                .upload_resumable(&local_path, &remote_path, resume, |transferred, _| {
                    progress(base.saturating_add(transferred), Some(total));
                })
                .await?;
            completed = completed.saturating_add(transferred);
        }

        Ok(TransferSummary {
            transferred: completed,
            skipped: tree.skipped,
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
        mut progress: F,
    ) -> Result<TransferSummary, AppError>
    where
        F: FnMut(u64, Option<u64>) + Send,
    {
        let matcher = filter.matcher()?;
        let tree = self.scan_remote_tree(remote_root, &matcher).await?;
        let total = tree.total_bytes();
        progress(0, Some(total));

        tokio::fs::create_dir_all(local_root)
            .await
            .map_err(|error| local_io_error("创建本地目录失败", local_root, error))?;
        for relative in &tree.directories {
            let local_path = local_path_for(local_root, relative)?;
            tokio::fs::create_dir_all(&local_path)
                .await
                .map_err(|error| local_io_error("创建本地目录失败", &local_path, error))?;
        }

        let mut completed = 0u64;
        for file in &tree.files {
            let remote_path = join_remote_path(remote_root, &file.relative);
            let local_path = local_path_for(local_root, &file.relative)?;
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
            completed = completed.saturating_add(transferred);
        }

        Ok(TransferSummary {
            transferred: completed,
            skipped: tree.skipped,
        })
    }

    async fn scan_remote_tree(
        &self,
        root: &str,
        matcher: &TransferMatcher,
    ) -> Result<ScannedTree, AppError> {
        let canonical = self.session.canonicalize(root).await.map_err(sftp_error)?;
        let mut tree = ScannedTree::default();
        let mut stack: Vec<(String, String)> = vec![(canonical, String::new())];
        while let Some((directory, relative_prefix)) = stack.pop() {
            let entries = self
                .session
                .read_dir(&directory)
                .await
                .map_err(sftp_error)?;
            let mut children = Vec::new();
            for entry in entries {
                let name = entry.file_name();
                let metadata = entry.metadata();
                children.push((
                    name,
                    metadata.is_dir(),
                    metadata.is_symlink(),
                    metadata.size.unwrap_or(0),
                ));
            }
            children.sort_by(|left, right| left.0.cmp(&right.0));

            for (name, is_dir, is_symlink, size) in children {
                if !safe_component(&name) || is_symlink {
                    tree.skipped = tree.skipped.saturating_add(1);
                    continue;
                }
                let relative = join_relative(&relative_prefix, &name);
                if matcher.matches(&relative, is_dir) {
                    tree.skipped = tree.skipped.saturating_add(1);
                    continue;
                }
                if is_dir {
                    tree.directories.push(relative.clone());
                    stack.push((join_remote_path(&directory, &name), relative));
                } else {
                    tree.files.push(ScannedFile { relative, size });
                }
            }
        }
        tree.sort();
        Ok(tree)
    }
}

async fn scan_local_tree(root: &Path, matcher: &TransferMatcher) -> Result<ScannedTree, AppError> {
    let metadata = tokio::fs::symlink_metadata(root)
        .await
        .map_err(|error| local_io_error("读取本地目录失败", root, error))?;
    if !metadata.is_dir() {
        return Err(AppError::with_detail(
            ErrorCode::SftpFailed,
            format!("本地路径不是目录: {}", root.display()),
        ));
    }

    let mut tree = ScannedTree::default();
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
                tree.skipped = tree.skipped.saturating_add(1);
                continue;
            }
            let file_type = match entry.file_type().await {
                Ok(file_type) => file_type,
                Err(_) => {
                    tree.skipped = tree.skipped.saturating_add(1);
                    continue;
                }
            };
            if file_type.is_symlink() {
                tree.skipped = tree.skipped.saturating_add(1);
                continue;
            }
            let relative = join_relative(&relative_prefix, &name);
            let is_dir = file_type.is_dir();
            if matcher.matches(&relative, is_dir) {
                tree.skipped = tree.skipped.saturating_add(1);
                continue;
            }
            if is_dir {
                tree.directories.push(relative.clone());
                stack.push((entry.path(), relative));
            } else {
                let size = entry
                    .metadata()
                    .await
                    .map(|metadata| metadata.len())
                    .unwrap_or(0);
                tree.files.push(ScannedFile { relative, size });
            }
        }
    }
    tree.sort();
    Ok(tree)
}

fn join_relative(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}/{name}")
    }
}

fn join_remote_path(root: &str, relative: &str) -> String {
    if root == "/" {
        format!("/{relative}")
    } else {
        format!("{}/{}", root.trim_end_matches('/'), relative)
    }
}

/// 将相对路径逐段拼接到本地根目录，并拒绝任何越界片段。
fn local_path_for(root: &Path, relative: &str) -> Result<PathBuf, AppError> {
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

fn safe_component(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains('/') && !name.contains('\\')
}

fn local_io_error(context: &str, path: &Path, error: std::io::Error) -> AppError {
    AppError::with_detail(
        ErrorCode::SftpFailed,
        format!("{context} {}: {error}", path.display()),
    )
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
