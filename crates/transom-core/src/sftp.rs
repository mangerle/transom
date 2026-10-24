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

/// 远端批量删除的单个目标；目录会按递归语义删除。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteDeleteTarget {
    pub path: String,
    pub is_dir: bool,
}

/// 远端文本的版本指纹：编辑面板打开或上次回传成功时记录，保存前与远端当前
/// 指纹比对，检测"打开后文件被他人修改"的并发冲突（丢失更新）。
///
/// 设计原理（Why）：stat-then-write 是乐观锁而非原子操作，比对与写入之间存在
/// 窗口；但它是 SFTP 协议下无需服务端配合的唯一可行方案，能拦住绝大多数
/// "打开到保存间隔数分钟"的真实冲突。代价与局限：mtime 精度为秒，同秒内同尺寸
/// 的修改无法识别；个别服务器不回传 mtime 时退化为仅按字节数比对。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RemoteTextMeta {
    /// 打开时或上次回传成功后的远端字节数。
    pub size: u64,
    /// 远端 mtime（秒）。`None` 表示服务器未回传，该维度视为通配不做比对，
    /// 避免偶发缺字段把合法保存误判成冲突。
    pub mtime: Option<u32>,
}

impl RemoteTextMeta {
    /// 与远端当前指纹不一致即视为冲突。
    ///
    /// 任一侧 mtime 缺失时跳过该维度，仅按字节数判定；两侧字节数都来自
    /// stat，缺失会直接跳过整个比对（见 net 层 `write_text`）。
    pub fn conflicts_with(&self, current: &RemoteTextMeta) -> bool {
        if self.size != current.size {
            return true;
        }
        matches!(
            (self.mtime, current.mtime),
            (Some(known), Some(now)) if known != now
        )
    }
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

/// 悬停预览的读取语义。
///
/// 设计原理（Why）：文本预览只需要文件头部若干字节即可立即出结果，
/// 图片预览必须拿到完整文件才能解码；用枚举区分两种读取边界，
/// 避免在调用点用布尔标志表达，防止调用方误把图片按头部截断读取后得到
/// 无法解码的字节。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewMode {
    /// 最多读取指定字节数，超出部分截断（文本预览）。
    Head,
    /// 必须完整读取；远端文件大于上限时直接返回错误（图片缩略图）。
    Full,
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
    /// 批量删除多个远端目标；由控制器逐个执行
    DeleteMany {
        targets: Vec<RemoteDeleteTarget>,
    },
    Chmod {
        path: String,
        permissions: u32,
    },
    /// 读取远端路径的详细属性（lstat 语义，不跟随符号链接）
    Stat {
        path: String,
    },
    /// 读取文件内容用于悬停轻量预览；只读、不进入传输队列。
    ///
    /// `max_bytes` 由调用方按预览类型给出上限：文本只读文件头，
    /// 图片要求完整读取并在超限时失败，避免为了一张缩略图把超大文件
    /// 全部拉入内存。
    Preview {
        path: String,
        mode: PreviewMode,
        max_bytes: u64,
    },
    /// 读取远端文本文件全文，供内置编辑器打开。
    ///
    /// 设计原理（Why）：与 `Preview` 一样走独立只读通道，**不进入传输队列**。
    /// 编辑取文件是应用内部行为，若混进用户队列会带来三类问题：占掉
    /// `max_concurrent_transfers` 的并发槽拖慢用户真正的批量传输、用户能取消或
    /// 清空掉在途的编辑下载（表现为编辑器莫名打不开）、以及完成统计把编辑算进
    /// "传输完成"。`max_bytes` 在读取循环内硬截断，防止目录列表尺寸过期时把
    /// 超限文件整份拉进内存。
    ReadText {
        path: String,
        max_bytes: u64,
    },
    /// 将编辑器缓冲区全文写回远端覆盖，走与 `ReadText` 对称的独立通道。
    ///
    /// `content` 为 UTF-8 字节；用 `Vec<u8>` 而非 `String` 是把编码决策留在
    /// 边界上，命令在通道里只被移动不被复制。
    ///
    /// `expected` 为打开或上次回传成功时的远端版本指纹（乐观锁）：写前先 stat
    /// 比对，不一致说明远端已被他人修改，拒绝覆盖并回报冲突。`None` 表示跳过
    /// 比对强制覆盖（用户在冲突横幅上明确选择"强制覆盖远端"时使用）。
    WriteText {
        path: String,
        content: Vec<u8>,
        expected: Option<RemoteTextMeta>,
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

    #[test]
    fn test_remote_text_meta_conflict_detection() {
        let opened = RemoteTextMeta {
            size: 100,
            mtime: Some(1000),
        };
        // 远端与打开时完全一致：不冲突。
        assert!(!opened.conflicts_with(&RemoteTextMeta {
            size: 100,
            mtime: Some(1000),
        }));
        // 大小变化（含保存后内容变短/变长）：冲突。
        assert!(opened.conflicts_with(&RemoteTextMeta {
            size: 90,
            mtime: Some(1000),
        }));
        // 大小相同但 mtime 变化（同尺寸被改写）：冲突。
        assert!(opened.conflicts_with(&RemoteTextMeta {
            size: 100,
            mtime: Some(2000),
        }));
    }

    #[test]
    fn test_remote_text_meta_treats_missing_mtime_as_wildcard() {
        // 打开时服务器未回传 mtime：只按大小比对，当前 mtime 出现不视为冲突，
        // 避免偶发缺字段把合法保存误判成冲突。
        let opened = RemoteTextMeta {
            size: 100,
            mtime: None,
        };
        assert!(!opened.conflicts_with(&RemoteTextMeta {
            size: 100,
            mtime: Some(2000),
        }));
        assert!(opened.conflicts_with(&RemoteTextMeta {
            size: 120,
            mtime: Some(2000),
        }));
        // 两侧 mtime 均缺失：退化为纯大小比对。
        assert!(!opened.conflicts_with(&RemoteTextMeta {
            size: 100,
            mtime: None,
        }));
    }
}
