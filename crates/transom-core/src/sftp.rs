use crate::ids::TransferId;
use crate::sftp_filter::TransferFilter;
use crate::sftp_sync::SyncItem;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteFile {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub size: u64,
    pub modified: Option<u32>,
    pub permissions: Option<u32>,
}

/// 远端文件/目录的详细属性快照（来自 SFTP lstat 与 readlink，用于属性查看弹窗）。
///
/// 设计原理（Why）：
/// - 实现初衷：SFTP 目录列表条目仅携带展示所需的最小字段，属性弹窗需要完整的
///   POSIX 元数据（uid/gid、访问时间、符号链接目标），故独立定义完整契约；
/// - 核心优势：可缺省字段全部使用 `Option` 兜底，不同服务端未返回的字段
///   只会降级展示，不会导致读取失败或界面中断；
/// - 代价与局限：部分服务端仅提供 uid/gid 数字或仅提供 user/group 名称，
///   展示层需要做“名称优先、数字兜底”的降级处理。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteMetadata {
    pub path: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub size: u64,
    pub uid: Option<u32>,
    pub user: Option<String>,
    pub gid: Option<u32>,
    pub group: Option<String>,
    pub permissions: Option<u32>,
    pub atime: Option<u32>,
    pub mtime: Option<u32>,
    pub link_target: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransferDirection {
    Download,
    Upload,
    /// 目录双向增量同步：单个任务内同时包含上传与下载项
    Sync,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TransferMode {
    #[default]
    Auto,
    Resume,
    Overwrite,
}

/// 传输目标类型：单文件或整棵目录树。
///
/// 目录传输在队列中只占一个任务，由后台逐项递归执行；`kind` 决定队列调度器
/// 选择单文件操作还是目录递归操作，避免根据路径实时猜测类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TransferKind {
    #[default]
    File,
    Directory,
    /// 目录对比后按差异计划双向同步
    Sync,
}

/// 目录递归传输的汇总结果。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TransferSummary {
    /// 实际完成（含断点续传已存在）的字节数
    pub transferred: u64,
    /// 被排除规则或符号链接跳过的条目数
    pub skipped: u64,
    /// 是否请求了 SHA256 校验且全部文件校验通过
    pub verified: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransferStatus {
    Pending,
    Transferring,
    Paused,
    Completed,
    Failed(String),
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferTask {
    pub id: TransferId,
    pub direction: TransferDirection,
    #[serde(default)]
    pub kind: TransferKind,
    pub local_path: PathBuf,
    pub remote_path: String,
    pub size: Option<u64>,
    pub transferred: u64,
    pub status: TransferStatus,
    pub mode: TransferMode,
    /// 同步任务携带的差异项；其他任务为空。
    #[serde(default)]
    pub sync_items: Vec<SyncItem>,
    /// 任务入队时是否要求传输完成后做 SHA256 校验。
    #[serde(default)]
    pub verify: bool,
}

impl TransferTask {
    pub fn new(
        direction: TransferDirection,
        local_path: PathBuf,
        remote_path: String,
        size: Option<u64>,
        mode: TransferMode,
    ) -> Self {
        Self {
            id: TransferId::new(),
            direction,
            kind: TransferKind::File,
            local_path,
            remote_path,
            size,
            transferred: 0,
            status: TransferStatus::Pending,
            mode,
            sync_items: Vec::new(),
            verify: false,
        }
    }

    /// 创建目录递归传输任务（队列中一个任务代表整棵目录树）。
    pub fn new_directory(
        direction: TransferDirection,
        local_path: PathBuf,
        remote_path: String,
        size: Option<u64>,
        mode: TransferMode,
    ) -> Self {
        let mut task = Self::new(direction, local_path, remote_path, size, mode);
        task.kind = TransferKind::Directory;
        task
    }

    /// 创建目录双向同步任务：`local_path`/`remote_path` 为两端根目录。
    pub fn new_sync(
        local_root: PathBuf,
        remote_root: String,
        items: Vec<SyncItem>,
        size: Option<u64>,
    ) -> Self {
        let mut task = Self::new(
            TransferDirection::Sync,
            local_root,
            remote_root,
            size,
            TransferMode::Overwrite,
        );
        task.kind = TransferKind::Sync;
        task.sync_items = items;
        task
    }
}

#[derive(Debug, Clone)]
pub enum SftpOperation {
    List {
        path: String,
    },
    Download {
        remote_path: String,
        local_path: PathBuf,
        resume: bool,
        verify: bool,
    },
    Upload {
        local_path: PathBuf,
        remote_path: String,
        resume: bool,
        verify: bool,
    },
    /// 递归上传整棵本地目录；`filter` 在遍历时剪枝被排除的子目录与文件。
    UploadDir {
        local_path: PathBuf,
        remote_path: String,
        resume: bool,
        filter: TransferFilter,
        verify: bool,
    },
    /// 递归下载整棵远端目录；`filter` 语义与 `UploadDir` 一致。
    DownloadDir {
        remote_path: String,
        local_path: PathBuf,
        resume: bool,
        filter: TransferFilter,
        verify: bool,
    },
    Mkdir {
        path: String,
    },
    Rename {
        from: String,
        to: String,
    },
    Delete {
        path: String,
        is_dir: bool,
    },
    Chmod {
        path: String,
        permissions: u32,
    },
    /// 读取远端路径的详细属性（lstat 语义，不跟随符号链接）
    Stat {
        path: String,
    },
    /// 创建远端软链接：`target` 为被指向的既有路径，`link_path` 为新建链接自身路径
    Symlink {
        target: String,
        link_path: String,
    },
    /// 对比本地与远端目录差异（只读扫描，不修改任何一侧）
    CompareDirs {
        local_path: PathBuf,
        remote_path: String,
        filter: TransferFilter,
    },
    /// 按用户确认的差异项执行双向增量同步
    SyncDirs {
        local_path: PathBuf,
        remote_path: String,
        items: Vec<SyncItem>,
        verify: bool,
    },
}

/// 将 Linux 文件权限掩码格式化为 3 位八进制字符串（如 `755`、`644`）。
///
/// 仅提取低 9 位（Owner/Group/Others 的 rwx 标志），避免文件类型高位干扰 UI 展示与编辑。
pub fn format_octal_permissions(permissions: u32) -> String {
    format!("{:03o}", permissions & 0o777)
}

/// 将 Linux 文件权限掩码格式化为 9 字符 `rwxr-xr-x` 形式。
pub fn format_rwx_permissions(permissions: u32) -> String {
    let mode = permissions & 0o777;
    let flags = [
        (0o400, 'r'),
        (0o200, 'w'),
        (0o100, 'x'),
        (0o040, 'r'),
        (0o020, 'w'),
        (0o010, 'x'),
        (0o004, 'r'),
        (0o002, 'w'),
        (0o001, 'x'),
    ];
    let mut out = String::with_capacity(9);
    for (mask, ch) in flags {
        out.push(if mode & mask != 0 { ch } else { '-' });
    }
    out
}

/// 解析用户输入的八进制权限字符串（如 `"755"` 或 `"0644"`）为 9 位权限掩码。
pub fn parse_octal_permissions(input: &str) -> Option<u32> {
    let trimmed = input.trim();
    if trimmed.is_empty() || trimmed.len() > 4 {
        return None;
    }
    let val = u32::from_str_radix(trimmed, 8).ok()?;
    if val <= 0o777 { Some(val) } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_octal_and_rwx_permissions() {
        assert_eq!(format_octal_permissions(0o100755), "755");
        assert_eq!(format_octal_permissions(0o100644), "644");
        assert_eq!(format_rwx_permissions(0o755), "rwxr-xr-x");
        assert_eq!(format_rwx_permissions(0o644), "rw-r--r--");
        assert_eq!(parse_octal_permissions("755"), Some(0o755));
        assert_eq!(parse_octal_permissions("0644"), Some(0o644));
        assert_eq!(parse_octal_permissions("999"), None);
        assert_eq!(parse_octal_permissions("1777"), None);
    }
}
