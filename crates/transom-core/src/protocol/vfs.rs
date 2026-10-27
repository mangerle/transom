//! 统一虚拟文件系统（VFS）抽象契约与数据模型。
//!
//! 设计原理（Why）：
//! - 实现初衷：消除本地文件浏览器与远端 SFTP 双栏在数据结构、列表项渲染、
//!   多选排序与文件系统操作契约上的双轨重复；
//! - 核心优势：
//!   1. `VfsEntry` 统一了本地与远端的文件项表现，UI 层无感知底层是操作系统还是 SFTP；
//!   2. `VirtualFileSystem` Trait 抽象了通用的目录遍历、属性读取、新建、删除与重命名能力；
//!   3. 支持无缝转换 `RemoteFile` 与本地路径元数据，向前兼容现有链路；
//! - 代价与局限：路径统一使用 `&str` / `String` 表达，本地 Provider 需要在 `PathBuf` 与
//!   UTF-8 路径字符串之间进行转换。

use crate::error::AppError;
use crate::protocol::sftp::RemoteFile;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// 虚拟文件系统通用文件条目。
///
/// 聚合本地与远端文件系统的共有属性，作为双栏浏览与多选拖拽的单一事实来源。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VfsEntry {
    /// 文件或目录名（如 `main.rs`、`photos`）。
    pub name: String,
    /// 完整规范路径（本地绝对路径或 SFTP 远端 POSIX 路径）。
    pub path: String,
    /// 是否为目录。
    pub is_dir: bool,
    /// 是否为符号链接。
    pub is_symlink: bool,
    /// 文件字节大小；目录通常为 0 或文件系统块大小。
    pub size: u64,
    /// 最后修改时间戳（UNIX 时间戳，秒）。
    pub modified: Option<u64>,
    /// POSIX 权限模式（9 位权限位，如 0o755）。
    pub permissions: Option<u32>,
}

impl VfsEntry {
    /// 构造一个新的虚拟文件系统条目。
    pub fn new(
        name: impl Into<String>,
        path: impl Into<String>,
        is_dir: bool,
        is_symlink: bool,
        size: u64,
    ) -> Self {
        Self {
            name: name.into(),
            path: path.into(),
            is_dir,
            is_symlink,
            size,
            modified: None,
            permissions: None,
        }
    }

    /// 设置修改时间戳。
    pub fn with_modified(mut self, modified: Option<u64>) -> Self {
        self.modified = modified;
        self
    }

    /// 设置权限位。
    pub fn with_permissions(mut self, permissions: Option<u32>) -> Self {
        self.permissions = permissions;
        self
    }

    /// 判断该条目是否为返回上一级目录的虚拟条目 `..`。
    pub fn is_parent_link(&self) -> bool {
        self.name == ".."
    }
}

impl From<RemoteFile> for VfsEntry {
    fn from(remote: RemoteFile) -> Self {
        Self {
            name: remote.name,
            path: remote.path,
            is_dir: remote.is_dir,
            is_symlink: remote.is_symlink,
            size: remote.size,
            modified: remote.modified.map(|m| m as u64),
            permissions: remote.permissions,
        }
    }
}

impl From<&RemoteFile> for VfsEntry {
    fn from(remote: &RemoteFile) -> Self {
        Self {
            name: remote.name.clone(),
            path: remote.path.clone(),
            is_dir: remote.is_dir,
            is_symlink: remote.is_symlink,
            size: remote.size,
            modified: remote.modified.map(|m| m as u64),
            permissions: remote.permissions,
        }
    }
}

impl From<VfsEntry> for RemoteFile {
    fn from(vfs: VfsEntry) -> Self {
        Self {
            name: vfs.name,
            path: vfs.path,
            is_dir: vfs.is_dir,
            is_symlink: vfs.is_symlink,
            size: vfs.size,
            modified: vfs.modified.map(|m| m as u32),
            permissions: vfs.permissions,
        }
    }
}

/// 虚拟文件系统操作契约。
///
/// 抽象文件浏览器的后端底层操作，本地文件系统与远端 SFTP 均可作为 Provider 实现。
#[async_trait]
pub trait VirtualFileSystem: Send + Sync {
    /// 列出指定目录下的全部子条目。
    async fn list_directory(&self, path: &str) -> Result<Vec<VfsEntry>, AppError>;

    /// 查询指定路径条目的元数据信息。
    async fn stat(&self, path: &str) -> Result<VfsEntry, AppError>;

    /// 创建新目录。
    async fn create_directory(&self, path: &str) -> Result<(), AppError>;

    /// 删除指定路径的文件或目录。
    async fn remove(&self, path: &str, is_dir: bool) -> Result<(), AppError>;

    /// 重命名或移动条目。
    async fn rename(&self, from: &str, to: &str) -> Result<(), AppError>;
}

/// 计算父目录路径（兼容 POSIX 与 Windows 格式）。
pub fn get_parent_path(path: &str) -> Option<String> {
    let p = Path::new(path);
    p.parent().and_then(|parent| {
        let parent_str = parent.to_str()?;
        if parent_str.is_empty() {
            None
        } else {
            Some(parent_str.to_string())
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vfs_entry_conversion_roundtrip() {
        let remote = RemoteFile {
            name: "config.yaml".to_string(),
            path: "/etc/config.yaml".to_string(),
            is_dir: false,
            is_symlink: false,
            size: 1024,
            modified: Some(1700000000),
            permissions: Some(0o644),
        };

        let entry: VfsEntry = remote.clone().into();
        assert_eq!(entry.name, "config.yaml");
        assert_eq!(entry.path, "/etc/config.yaml");
        assert_eq!(entry.size, 1024);
        assert!(!entry.is_dir);
        assert!(!entry.is_parent_link());

        let roundtrip: RemoteFile = entry.into();
        assert_eq!(roundtrip, remote);
    }

    #[test]
    fn parent_path_resolution() {
        assert_eq!(
            get_parent_path("/home/user/test"),
            Some("/home/user".to_string())
        );
        assert_eq!(get_parent_path("/home/user"), Some("/home".to_string()));
    }
}
