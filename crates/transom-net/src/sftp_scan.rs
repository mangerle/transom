//! 目录树递归扫描：为目录传输、对比与同步提供统一的文件清单。
//!
//! 设计原理（Why）：
//! - 实现初衷：远端扫描每个目录一次网络往返，串行走完整棵树的时间与目录数
//!   成正比（node_modules 级别的目录树在高延迟链路上可达数十分钟）；
//! - 核心优势：批式工作列（每轮取固定数量的目录并发读取）实现简单、内存
//!   有界，且不依赖递归 future，树深不影响栈；扫描是只读操作，批内乱序
//!   完成不影响结果，最终统一按相对路径排序保证输出与串行实现一致；
//! - 代价与局限：错误在批次间按提交顺序返回，同批失败目录的兄弟目录仍会
//!   完成读取（多余但无害）；并发度对单连接的请求压力受服务器实现约束，
//!   固定为 8 以平衡吞吐与过载风险。

use crate::sftp::{SftpClient, sftp_error};
use crate::sftp_dir::{join_remote_path, local_io_error, safe_component};
use futures::stream::{self, StreamExt};
use std::path::{Path, PathBuf};
use transom_core::{AppError, DirEntryMeta, DirSnapshotEntry, TransferMatcher};

/// 远端目录扫描的单批并发 read_dir 数。
const REMOTE_SCAN_CONCURRENCY: usize = 8;

/// 本地目录扫描的单批并发 read_dir 数。
/// 本地磁盘虽无网络往返，但网络挂载（FUSE/sshfs/NFS）同样存在每次打开的
/// 等待，并发读取可整体提速。
const LOCAL_SCAN_CONCURRENCY: usize = 8;

/// 远端单个目录的紧凑条目快照：(名称, 是否目录, 是否链接, 大小, 修改时间)。
type RemoteChild = (String, bool, bool, u64, Option<u32>);

/// 单侧目录扫描结果。
pub(crate) struct ScanResult {
    pub(crate) entries: Vec<DirSnapshotEntry>,
    pub(crate) skipped: u64,
}

impl ScanResult {
    /// 汇总所有文件条目的字节数（目录不计入），全程饱和累加防溢出。
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
    /// 递归扫描远端目录，供目录传输、对比与同步使用。
    ///
    /// 安全约束：远端返回的文件名参与相对路径拼接前必须通过
    /// [`safe_component`] 校验，非法片段计入跳过而不是中断扫描。
    ///
    /// # Errors
    /// 根路径规范化或任一目录读取失败时返回 `ErrorCode::SftpFailed`，
    /// 与历史串行实现的首错即返语义一致。
    pub(crate) async fn scan_remote_snapshot(
        &self,
        root: &str,
        matcher: &TransferMatcher,
    ) -> Result<ScanResult, AppError> {
        let canonical = self.session.canonicalize(root).await.map_err(sftp_error)?;
        let mut entries = Vec::new();
        let mut skipped = 0u64;
        let mut pending: Vec<(String, String)> = vec![(canonical, String::new())];
        while !pending.is_empty() {
            let batch: Vec<(String, String)> = pending
                .drain(..pending.len().min(REMOTE_SCAN_CONCURRENCY))
                .collect();
            let results = stream::iter(batch)
                .map(|(directory, relative_prefix)| async move {
                    let children = self
                        .session
                        .read_dir(&directory)
                        .await
                        .map_err(sftp_error)?;
                    let mut compact: Vec<RemoteChild> = Vec::new();
                    for entry in children {
                        let metadata = entry.metadata();
                        compact.push((
                            entry.file_name(),
                            metadata.is_dir(),
                            metadata.is_symlink(),
                            metadata.size.unwrap_or(0),
                            metadata.mtime,
                        ));
                    }
                    compact.sort_by(|left, right| left.0.cmp(&right.0));
                    Ok::<(String, String, Vec<RemoteChild>), AppError>((
                        directory,
                        relative_prefix,
                        compact,
                    ))
                })
                .buffer_unordered(REMOTE_SCAN_CONCURRENCY)
                .collect::<Vec<_>>()
                .await;
            for result in results {
                let (directory, relative_prefix, children) = result?;
                for (name, is_dir, is_symlink, size, mtime) in children {
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
                        pending.push((join_remote_path(&directory, &name), relative));
                    }
                }
            }
        }
        entries.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
        Ok(ScanResult { entries, skipped })
    }
}

/// 递归扫描本地目录，供目录传输、对比与同步使用。
///
/// 与远端扫描同一批式并发结构；本地条目额外做一次 `symlink_metadata` 探测，
/// 符号链接一律计入跳过，防止链接环导致扫描不终止。
///
/// # Errors
/// 根路径不存在、不是目录或任一子目录读取失败时返回 `ErrorCode::SftpFailed`。
pub(crate) async fn scan_local_snapshot(
    root: &Path,
    matcher: &TransferMatcher,
) -> Result<ScanResult, AppError> {
    let metadata = tokio::fs::symlink_metadata(root)
        .await
        .map_err(|error| local_io_error("读取本地目录失败", root, error))?;
    if !metadata.is_dir() {
        return Err(AppError::with_detail(
            transom_core::ErrorCode::SftpFailed,
            format!("本地路径不是目录: {}", root.display()),
        ));
    }

    let mut entries = Vec::new();
    let mut skipped = 0u64;
    let mut pending: Vec<(PathBuf, String)> = vec![(root.to_path_buf(), String::new())];
    while !pending.is_empty() {
        let batch: Vec<(PathBuf, String)> = pending
            .drain(..pending.len().min(LOCAL_SCAN_CONCURRENCY))
            .collect();
        let results = stream::iter(batch)
            .map(|(directory, relative_prefix)| async move {
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
                Ok::<(PathBuf, String, Vec<tokio::fs::DirEntry>), AppError>((
                    directory,
                    relative_prefix,
                    children,
                ))
            })
            .buffer_unordered(LOCAL_SCAN_CONCURRENCY)
            .collect::<Vec<_>>()
            .await;
        for result in results {
            let (_directory, relative_prefix, children) = result?;
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
                    pending.push((entry.path(), relative));
                }
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

/// 文件条目按相对路径排序。
pub(crate) fn sorted_files(entries: &[DirSnapshotEntry]) -> Vec<&DirSnapshotEntry> {
    let mut files = entries
        .iter()
        .filter(|entry| !entry.meta.is_dir)
        .collect::<Vec<_>>();
    files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    files
}

/// 把排序后的目录相对路径按层级深度切成若干层，层内目录互为父子不可能出现，
/// 可安全并发创建；层与层之间保持先后，父目录先于全部子目录就绪。
pub(crate) fn layered_directories(entries: &[DirSnapshotEntry]) -> Vec<Vec<&str>> {
    let sorted = sorted_directories(entries);
    let mut layers: Vec<Vec<&str>> = Vec::new();
    for path in sorted {
        let depth = path.matches('/').count();
        let needs_new_layer = layers
            .last()
            .is_none_or(|layer| layer[0].matches('/').count() != depth);
        if needs_new_layer {
            layers.push(vec![path]);
        } else if let Some(layer) = layers.last_mut() {
            layer.push(path);
        }
    }
    layers
}

/// 拼接扫描相对路径：空前缀直接用名称，否则以 `/` 连接。
pub(crate) fn join_relative(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}/{name}")
    }
}

/// SystemTime 转 Unix 秒，超出 u32 范围截断到上限。
pub(crate) fn unix_seconds(time: Option<std::time::SystemTime>) -> Option<u32> {
    let seconds = time?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    Some(seconds.min(u64::from(u32::MAX)) as u32)
}

#[cfg(test)]
mod tests {
    use super::{join_relative, layered_directories};
    use transom_core::{DirEntryMeta, DirSnapshotEntry};

    fn entry(relative: &str, is_dir: bool) -> DirSnapshotEntry {
        DirSnapshotEntry {
            relative_path: relative.to_string(),
            meta: DirEntryMeta {
                is_dir,
                size: 0,
                mtime: None,
            },
        }
    }

    #[test]
    fn layered_directories_groups_by_depth() {
        let entries = vec![
            entry("a/b", true),
            entry("a", true),
            entry("a/b/c", true),
            entry("c", true),
            entry("a/f.txt", false),
        ];
        let layers = layered_directories(&entries);
        // 每层内深度一致且父目录所在层先于子目录所在层
        assert_eq!(layers, vec![vec!["a", "c"], vec!["a/b"], vec!["a/b/c"]]);
    }

    #[test]
    fn join_relative_handles_empty_prefix() {
        assert_eq!(join_relative("", "name"), "name");
        assert_eq!(join_relative("a/b", "c"), "a/b/c");
    }
}
