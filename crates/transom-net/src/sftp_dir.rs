//! SFTP 目录递归传输：目录树扫描结果（见 `sftp_scan`）之上的建目录与逐文件传输。
//!
//! 传输阶段的三处并行化：
//! - 远端/本地目录树扫描在 `sftp_scan` 中按批次并发读取；
//! - 上传建目录按层级分层，层内并发 `ensure_dir`；
//! - 文件传输以固定并发度并行，进度经原子计数聚合后统一汇报。

use crate::sftp::{SftpClient, notify_shared_progress, sftp_error};
use crate::sftp_scan::{layered_directories, scan_local_snapshot, sorted_files};
use futures::stream::{self, StreamExt, TryStreamExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use transom_core::{AppError, ErrorCode, TransferFilter, TransferSummary};

/// 并发目录传输的聚合进度回调：装箱为具体类型，供嵌套闭包安全捕获。
type SharedProgress = Arc<std::sync::Mutex<Box<dyn FnMut(u64, Option<u64>) + Send>>>;

/// 建目录阶段每层内的并发 ensure_dir 数。
const DIR_MKDIR_CONCURRENCY: usize = 8;

/// 文件传输阶段的单文件并发数。
/// 海量小文件场景下单文件串行每次至少 3 个往返（打开、读写、关闭），
/// 并发 8 可把总时间压到约 1/8；单个大文件内部由 russh-sftp 的深预取
/// （见 `sftp` 模块）摊平往返，两者叠加后单连接整体保持适中压力，
/// 不触发服务器过载保护。
const DIR_FILE_CONCURRENCY: usize = 8;

impl SftpClient {
    /// 递归删除远端目录：先收集整棵子树，再逐文件、自底向上逐目录删除。
    ///
    /// 设计原理（Why）：
    /// - SFTP 的 `RMDIR` 与 POSIX `rmdir` 一致，只能删除空目录；
    ///   用户视角的“删除目录”需要自行遍历并清理后代；
    /// - 使用显式栈迭代而非递归，避免超深目录树导致栈溢出；
    /// - 符号链接只删除链接本身，绝不跟随链接进入目标目录，防止误删链接
    ///   指向的其他目录；
    /// - 收集完成后再统一删除，遍历阶段不会因边删边读导致目录项遗漏。
    ///
    /// # Errors
    /// 根目录非法、读取目录、删除文件或删除目录失败时返回
    /// `ErrorCode::SftpFailed`；已完成的部分删除不会回滚。
    pub(crate) async fn delete_dir_recursive(&self, root: &str) -> Result<(), AppError> {
        let canonical = self.session.canonicalize(root).await.map_err(sftp_error)?;
        if canonical == "/" {
            return Err(AppError::with_detail(
                ErrorCode::SftpFailed,
                "不允许递归删除远端根目录",
            ));
        }

        let mut stack = vec![canonical];
        let mut directories = Vec::new();
        let mut files = Vec::new();
        while let Some(directory) = stack.pop() {
            directories.push(directory.clone());
            let entries = self.session.read_dir(&directory).await.map_err(|error| {
                AppError::with_detail(
                    ErrorCode::SftpFailed,
                    format!("读取待删除目录失败 {directory}: {error}"),
                )
            })?;
            for entry in entries {
                let name = entry.file_name();
                if name == "." || name == ".." {
                    continue;
                }
                if !safe_component(&name) {
                    return Err(AppError::with_detail(
                        ErrorCode::SftpFailed,
                        format!("待删除目录包含非法名称: {name}"),
                    ));
                }
                let path = join_remote_path(&directory, &name);
                let metadata = entry.metadata();
                if metadata.is_symlink() || !metadata.is_dir() {
                    files.push(path);
                } else {
                    stack.push(path);
                }
            }
        }

        for file in files {
            self.session.remove_file(&file).await.map_err(|error| {
                AppError::with_detail(
                    ErrorCode::SftpFailed,
                    format!("删除远端文件失败 {file}: {error}"),
                )
            })?;
        }
        // 发现顺序保证父目录先于子目录入栈，逆序即可自底向上删除
        for directory in directories.into_iter().rev() {
            self.session.remove_dir(&directory).await.map_err(|error| {
                AppError::with_detail(
                    ErrorCode::SftpFailed,
                    format!("删除远端目录失败 {directory}: {error}"),
                )
            })?;
        }
        Ok(())
    }

    /// 递归上传本地目录，返回整体字节数与跳过条目数。
    ///
    /// 设计原理（Why）：
    /// - 实现初衷：目录传输需要先建立完整文件清单才能给出准确总进度，因此在
    ///   开始传输前完成一次只读扫描；
    /// - 核心优势：排除规则在扫描阶段对目录直接剪枝，`.git` 等大目录不会被
    ///   逐文件遍历；扫描结果同时用于预创建空目录，保证空目录也能同步；
    ///   建目录按层级分层、层内并发，文件按固定并发度并行传输，海量小文件
    ///   目录的总耗时不再与"文件数 × 单文件往返数"成正比；
    /// - 代价与局限：扫描与传输之间如果文件被外部修改，总进度可能略有偏差，
    ///   因此进度计算全部使用饱和算术，不追求严格字节守恒；任一文件失败即
    ///   停止调度后续文件（与串行实现的首错即断一致），已完成的文件不回滚。
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
        progress: F,
    ) -> Result<TransferSummary, AppError>
    where
        F: FnMut(u64, Option<u64>) + Send + 'static,
    {
        let matcher = filter.matcher()?;
        let scan = scan_local_snapshot(local_root, &matcher).await?;
        let total = scan.total_bytes();
        let transferred = Arc::new(AtomicU64::new(0));
        // 回调装箱为具体类型，避免泛型闭包深入并发任务（理由见 DirFileTask 文档）
        let progress: SharedProgress = Arc::new(std::sync::Mutex::new(Box::new(progress)));
        notify_shared_progress(&progress, 0, Some(total));

        self.create_dir_all(remote_root).await?;
        // 目录按深度分层：层内互不为父子可并发 ensure_dir，层间保证父目录先就绪
        for layer in layered_directories(&scan.entries) {
            let paths: Vec<String> = layer
                .iter()
                .map(|relative| join_remote_path(remote_root, relative))
                .collect();
            let ensures: Vec<_> = paths.iter().map(|path| self.ensure_dir(path)).collect();
            let results = stream::iter(ensures)
                .buffer_unordered(DIR_MKDIR_CONCURRENCY)
                .collect::<Vec<_>>()
                .await;
            for result in results {
                result?;
            }
        }

        let files = sorted_files(&scan.entries);
        let mut tasks = Vec::with_capacity(files.len());
        for file in files {
            tasks.push(DirFileTask {
                client: self.clone(),
                local_path: local_path_for(local_root, &file.relative_path)?,
                remote_path: join_remote_path(remote_root, &file.relative_path),
                resume,
                verify,
                total,
                transferred: transferred.clone(),
                progress: progress.clone(),
            });
        }
        stream::iter(tasks)
            .map(|task| task.run_upload())
            .buffer_unordered(DIR_FILE_CONCURRENCY)
            .try_collect::<Vec<()>>()
            .await?;

        Ok(TransferSummary {
            transferred: transferred.load(Ordering::Relaxed),
            skipped: scan.skipped,
            verified: verify,
        })
    }

    /// 递归下载远端目录，返回整体字节数与跳过条目数。
    ///
    /// 安全约束：远端返回的文件名会参与本地路径拼接，任何空片段、`.`、`..`
    /// 或路径分隔符都会被拒绝，防止恶意服务端把文件写到目标目录之外。
    /// 文件按固定并发度并行下载（与上传同构），进度经原子计数聚合。
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
        progress: F,
    ) -> Result<TransferSummary, AppError>
    where
        F: FnMut(u64, Option<u64>) + Send + 'static,
    {
        let matcher = filter.matcher()?;
        let scan = self.scan_remote_snapshot(remote_root, &matcher).await?;
        let total = scan.total_bytes();
        let transferred = Arc::new(AtomicU64::new(0));
        // 回调装箱为具体类型，避免泛型闭包深入并发任务（理由见 DirFileTask 文档）
        let progress: SharedProgress = Arc::new(std::sync::Mutex::new(Box::new(progress)));
        notify_shared_progress(&progress, 0, Some(total));

        // 本地 create_dir_all 自带父目录创建且无网络往返，顺序执行即可
        tokio::fs::create_dir_all(local_root)
            .await
            .map_err(|error| local_io_error("创建本地目录失败", local_root, error))?;
        for entry in scan.entries.iter().filter(|entry| entry.meta.is_dir) {
            let local_path = local_path_for(local_root, &entry.relative_path)?;
            tokio::fs::create_dir_all(&local_path)
                .await
                .map_err(|error| local_io_error("创建本地目录失败", &local_path, error))?;
        }

        let files = sorted_files(&scan.entries);
        let mut tasks = Vec::with_capacity(files.len());
        for file in files {
            tasks.push(DirFileTask {
                client: self.clone(),
                local_path: local_path_for(local_root, &file.relative_path)?,
                remote_path: join_remote_path(remote_root, &file.relative_path),
                resume,
                verify,
                total,
                transferred: transferred.clone(),
                progress: progress.clone(),
            });
        }
        stream::iter(tasks)
            .map(|task| task.run_download())
            .buffer_unordered(DIR_FILE_CONCURRENCY)
            .try_collect::<Vec<()>>()
            .await?;

        Ok(TransferSummary {
            transferred: transferred.load(Ordering::Relaxed),
            skipped: scan.skipped,
            verified: verify,
        })
    }

    /// 跨远端递归传输整棵目录树。
    ///
    /// # Errors
    /// 排除规则非法、源端目录读取失败、目标端建目录或跨远端流式写入失败时返回 `ErrorCode::SftpFailed`。
    pub async fn transfer_dir_to_remote<F>(
        &self,
        source_root: &str,
        target_sftp: &SftpClient,
        target_root: &str,
        filter: &TransferFilter,
        preserve_attributes: bool,
        progress: F,
    ) -> Result<TransferSummary, AppError>
    where
        F: FnMut(u64, Option<u64>) + Send + 'static,
    {
        let matcher = filter.matcher()?;
        let scan = self.scan_remote_snapshot(source_root, &matcher).await?;
        let total = scan.total_bytes();
        let transferred = Arc::new(AtomicU64::new(0));
        let progress: SharedProgress = Arc::new(std::sync::Mutex::new(Box::new(progress)));
        notify_shared_progress(&progress, 0, Some(total));

        target_sftp.create_dir_all(target_root).await?;
        for layer in layered_directories(&scan.entries) {
            let paths: Vec<String> = layer
                .iter()
                .map(|relative| join_remote_path(target_root, relative))
                .collect();
            let ensures: Vec<_> = paths
                .iter()
                .map(|path| target_sftp.ensure_dir(path))
                .collect();
            let results = stream::iter(ensures)
                .buffer_unordered(DIR_MKDIR_CONCURRENCY)
                .collect::<Vec<_>>()
                .await;
            for result in results {
                result?;
            }
        }

        let files = sorted_files(&scan.entries);
        let mut tasks = Vec::with_capacity(files.len());
        for file in files {
            tasks.push(DirRemoteCopyTask {
                source_client: self.clone(),
                target_client: target_sftp.clone(),
                source_path: join_remote_path(source_root, &file.relative_path),
                target_path: join_remote_path(target_root, &file.relative_path),
                preserve_attributes,
                total,
                transferred: transferred.clone(),
                progress: progress.clone(),
            });
        }
        stream::iter(tasks)
            .map(|task| task.run_remote_copy())
            .buffer_unordered(DIR_FILE_CONCURRENCY)
            .try_collect::<Vec<()>>()
            .await?;

        Ok(TransferSummary {
            transferred: transferred.load(Ordering::Relaxed),
            skipped: scan.skipped,
            verified: false,
        })
    }
}

/// 跨远端目录传输单个文件的并发任务上下文。
struct DirRemoteCopyTask {
    source_client: SftpClient,
    target_client: SftpClient,
    source_path: String,
    target_path: String,
    preserve_attributes: bool,
    total: u64,
    transferred: Arc<AtomicU64>,
    progress: SharedProgress,
}

impl DirRemoteCopyTask {
    async fn run_remote_copy(self) -> Result<(), AppError> {
        let mut last_reported = 0u64;
        let bytes = self
            .source_client
            .transfer_to_remote(
                &self.source_path,
                &self.target_client,
                &self.target_path,
                self.preserve_attributes,
                |current, _| {
                    let delta = current.saturating_sub(last_reported);
                    last_reported = current;
                    if delta > 0 {
                        let done = self.transferred.fetch_add(delta, Ordering::Relaxed) + delta;
                        notify_shared_progress(&self.progress, done, Some(self.total));
                    }
                },
            )
            .await?;
        let tail = bytes.saturating_sub(last_reported);
        if tail > 0 {
            let done = self.transferred.fetch_add(tail, Ordering::Relaxed) + tail;
            notify_shared_progress(&self.progress, done, Some(self.total));
        }
        Ok(())
    }
}

/// 目录传输单个文件的并发任务上下文。
///
/// 设计原理（Why）：
/// - 实现初衷：目录传输的回调是调用方的泛型闭包，若让它经"外层闭包包
///   async 块"多级嵌套再被 `tokio::spawn` 要求 `'static`，会触发 rustc
///   的 HRTT 推断限制（FnOnce is not general enough）；
/// - 核心优势：入口处把回调装箱为具体类型并连同参数收进本结构，任务体是
///   具名 async 函数，结构体内不再出现闭包嵌套闭包，完全绕开该限制；
/// - 代价与局限：一次堆装箱（整个目录传输仅一次），换取可编译且清晰的
///   任务边界。
struct DirFileTask {
    client: SftpClient,
    local_path: PathBuf,
    remote_path: String,
    resume: bool,
    verify: bool,
    total: u64,
    transferred: Arc<AtomicU64>,
    progress: SharedProgress,
}

impl DirFileTask {
    /// 上传单个文件：字节增量聚合进共享计数，完成后按需校验并补齐尾部进度。
    async fn run_upload(self) -> Result<(), AppError> {
        let mut last_reported = 0u64;
        let bytes = self
            .client
            .upload_resumable(
                &self.local_path,
                &self.remote_path,
                self.resume,
                |current, _| {
                    let delta = current.saturating_sub(last_reported);
                    last_reported = current;
                    if delta > 0 {
                        let done = self.transferred.fetch_add(delta, Ordering::Relaxed) + delta;
                        notify_shared_progress(&self.progress, done, Some(self.total));
                    }
                },
            )
            .await?;
        if self.verify {
            self.client
                .verify_file(&self.local_path, &self.remote_path)
                .await?;
        }
        // 进度回调只覆盖到最近的块边界，剩余字节在完成后补齐
        let tail = bytes.saturating_sub(last_reported);
        if tail > 0 {
            let done = self.transferred.fetch_add(tail, Ordering::Relaxed) + tail;
            notify_shared_progress(&self.progress, done, Some(self.total));
        }
        Ok(())
    }

    /// 下载单个文件：确保本地父目录存在后拉取，进度聚合与上传同构。
    async fn run_download(self) -> Result<(), AppError> {
        if let Some(parent) = self.local_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|error| local_io_error("创建本地父目录失败", parent, error))?;
        }
        let mut last_reported = 0u64;
        let bytes = self
            .client
            .download_resumable(
                &self.remote_path,
                &self.local_path,
                self.resume,
                |current, _| {
                    let delta = current.saturating_sub(last_reported);
                    last_reported = current;
                    if delta > 0 {
                        let done = self.transferred.fetch_add(delta, Ordering::Relaxed) + delta;
                        notify_shared_progress(&self.progress, done, Some(self.total));
                    }
                },
            )
            .await?;
        if self.verify {
            self.client
                .verify_file(&self.local_path, &self.remote_path)
                .await?;
        }
        let tail = bytes.saturating_sub(last_reported);
        if tail > 0 {
            let done = self.transferred.fetch_add(tail, Ordering::Relaxed) + tail;
            notify_shared_progress(&self.progress, done, Some(self.total));
        }
        Ok(())
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
