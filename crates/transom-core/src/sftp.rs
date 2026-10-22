use crate::ids::TransferId;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransferDirection {
    Download,
    Upload,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TransferMode {
    #[default]
    Auto,
    Resume,
    Overwrite,
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
    pub local_path: PathBuf,
    pub remote_path: String,
    pub size: Option<u64>,
    pub transferred: u64,
    pub status: TransferStatus,
    pub mode: TransferMode,
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
            local_path,
            remote_path,
            size,
            transferred: 0,
            status: TransferStatus::Pending,
            mode,
        }
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
    },
    Upload {
        local_path: PathBuf,
        remote_path: String,
        resume: bool,
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
