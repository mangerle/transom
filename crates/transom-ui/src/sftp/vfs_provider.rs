//! 本地文件系统 VirtualFileSystem Provider 实现。
//!
//! 设计原理（Why）：
//! - 实现初衷：为本地文件操作提供符合统一 `VirtualFileSystem` 契约的标准 Provider，
//!   消除与远端 SFTP 双轨并行的特化逻辑；
//! - 核心优势：
//!   1. 统一产出强类型 `VfsEntry`，自动完成目录优先、不区分大小写字典序排序；
//!   2. 抽象了本地文件系统的基础 I/O 操作（新建、删除、重命名、属性）；
//!   3. 方便测试打桩与未来接入其他非本地/沙箱文件系统；
//! - 代价与局限：本地文件 I/O 涉及阻塞系统调用，高层应通过后台执行器（如 `spawn_blocking`）调度。

use std::path::Path;
use transom_core::{AppError, ErrorCode, VfsEntry, VirtualFileSystem, async_trait};

/// 本地文件系统 VFS 接入实现。
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct LocalVfsProvider;

impl LocalVfsProvider {
    /// 同步扫描本地目录并返回排序后的 VFS 条目列表。
    pub(crate) fn list_sync(dir: &Path) -> Result<Vec<VfsEntry>, AppError> {
        let read_dir = std::fs::read_dir(dir).map_err(|error| {
            AppError::with_detail(ErrorCode::Io, format!("读取本地目录失败: {error}"))
        })?;
        let mut entries = Vec::new();
        for item in read_dir.flatten() {
            let path = item.path();
            let path_str = path.to_string_lossy().to_string();
            let name = item.file_name().to_string_lossy().to_string();
            let file_type = item.file_type().ok();
            let is_dir = file_type.as_ref().map(|ft| ft.is_dir()).unwrap_or(false);
            let is_symlink = file_type
                .as_ref()
                .map(|ft| ft.is_symlink())
                .unwrap_or(false);
            let metadata = item.metadata().ok();
            let size = if is_dir {
                0
            } else {
                metadata.as_ref().map(|m| m.len()).unwrap_or(0)
            };
            let modified = metadata
                .as_ref()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs());

            let mut entry = VfsEntry::new(name, path_str, is_dir, is_symlink, size);
            entry.modified = modified;
            entries.push(entry);
        }

        // 统一排序：目录优先，随后按名称小写字母序排序
        entries.sort_by(|a, b| {
            b.is_dir.cmp(&a.is_dir).then_with(|| {
                a.name
                    .to_ascii_lowercase()
                    .cmp(&b.name.to_ascii_lowercase())
            })
        });
        Ok(entries)
    }
}

#[async_trait]
impl VirtualFileSystem for LocalVfsProvider {
    async fn list_directory(&self, path: &str) -> Result<Vec<VfsEntry>, AppError> {
        let dir = Path::new(path).to_path_buf();
        tokio::task::spawn_blocking(move || Self::list_sync(&dir))
            .await
            .map_err(|e| AppError::with_detail(ErrorCode::Io, format!("调度读取失败: {e}")))?
    }

    async fn stat(&self, path: &str) -> Result<VfsEntry, AppError> {
        let p = Path::new(path).to_path_buf();
        tokio::task::spawn_blocking(move || {
            let metadata = std::fs::symlink_metadata(&p).map_err(|e| {
                AppError::with_detail(ErrorCode::Io, format!("查询元数据失败: {e}"))
            })?;
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| p.to_string_lossy().to_string());
            let is_dir = metadata.is_dir();
            let is_symlink = metadata.file_type().is_symlink();
            let size = if is_dir { 0 } else { metadata.len() };
            let modified = metadata
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs());

            let mut entry = VfsEntry::new(
                name,
                p.to_string_lossy().to_string(),
                is_dir,
                is_symlink,
                size,
            );
            entry.modified = modified;
            Ok(entry)
        })
        .await
        .map_err(|e| AppError::with_detail(ErrorCode::Io, format!("调度查询失败: {e}")))?
    }

    async fn create_directory(&self, path: &str) -> Result<(), AppError> {
        let p = Path::new(path).to_path_buf();
        tokio::task::spawn_blocking(move || {
            std::fs::create_dir_all(&p)
                .map_err(|e| AppError::with_detail(ErrorCode::Io, format!("新建目录失败: {e}")))
        })
        .await
        .map_err(|e| AppError::with_detail(ErrorCode::Io, format!("调度新建目录失败: {e}")))?
    }

    async fn remove(&self, path: &str, is_dir: bool) -> Result<(), AppError> {
        let p = Path::new(path).to_path_buf();
        tokio::task::spawn_blocking(move || {
            if is_dir {
                std::fs::remove_dir_all(&p)
                    .map_err(|e| AppError::with_detail(ErrorCode::Io, format!("删除目录失败: {e}")))
            } else {
                std::fs::remove_file(&p)
                    .map_err(|e| AppError::with_detail(ErrorCode::Io, format!("删除文件失败: {e}")))
            }
        })
        .await
        .map_err(|e| AppError::with_detail(ErrorCode::Io, format!("调度删除失败: {e}")))?
    }

    async fn rename(&self, from: &str, to: &str) -> Result<(), AppError> {
        let from_p = Path::new(from).to_path_buf();
        let to_p = Path::new(to).to_path_buf();
        tokio::task::spawn_blocking(move || {
            std::fs::rename(&from_p, &to_p)
                .map_err(|e| AppError::with_detail(ErrorCode::Io, format!("重命名失败: {e}")))
        })
        .await
        .map_err(|e| AppError::with_detail(ErrorCode::Io, format!("调度重命名失败: {e}")))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_vfs_provider_lists_current_dir() {
        let _provider = LocalVfsProvider;
        let entries = LocalVfsProvider::list_sync(Path::new(".")).unwrap();
        assert!(!entries.is_empty());
        // 验证目录排在文件前面
        let first_file_idx = entries.iter().position(|e| !e.is_dir);
        let last_dir_idx = entries.iter().rposition(|e| e.is_dir);
        if let (Some(f), Some(d)) = (first_file_idx, last_dir_idx) {
            assert!(d < f, "目录应当排列在普通文件前面");
        }
    }
}
