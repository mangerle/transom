//! SFTP 业务操作枚举定义。

use super::transfer::{ChecksumAlgorithm, VerifyMode};
use super::types::{RemoteDeleteTarget, RemoteTextMeta};
use crate::ids::ConnectionId;
use crate::protocol::sftp_filter::TransferFilter;
use crate::protocol::sftp_sync::SyncItem;
use std::path::PathBuf;

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
        mode: VerifyMode,
        algorithm: ChecksumAlgorithm,
    },
    Upload {
        local_path: PathBuf,
        remote_path: String,
        resume: bool,
        verify: bool,
        mode: VerifyMode,
        algorithm: ChecksumAlgorithm,
    },
    /// 递归上传整棵本地目录；`filter` 在遍历时剪枝被排除的子目录与文件。
    UploadDir {
        local_path: PathBuf,
        remote_path: String,
        resume: bool,
        filter: TransferFilter,
        verify: bool,
        mode: VerifyMode,
        algorithm: ChecksumAlgorithm,
    },
    /// 递归下载整棵远端目录；`filter` 语义与 `UploadDir` 一致。
    DownloadDir {
        remote_path: String,
        local_path: PathBuf,
        resume: bool,
        filter: TransferFilter,
        verify: bool,
        mode: VerifyMode,
        algorithm: ChecksumAlgorithm,
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
    /// `expected` 为打开或上次回传成功时的远端版本指纹（乐观锁）：net 层写前先
    /// stat 比对，不一致说明远端已被他人修改，拒绝覆盖并回报冲突；同时按指纹的
    /// 形态标记还原 BOM/CRLF。`None` 表示新文件（远端尚不存在），不做比对、
    /// 不做还原。
    ///
    /// `force` 为 true 时跳过指纹比对（用户在冲突横幅上明确选择"强制覆盖远端"），
    /// 但形态还原仍然生效，避免强制覆盖悄悄改掉文件的 BOM/CRLF。
    ///
    /// `backup` 为 true 时先把远端当前内容备份为 `{path}.transom-bak` 再覆盖
    /// （由设置项下发）；备份属尽力而为，远端不存在或读取失败时跳过。
    WriteText {
        path: String,
        content: Vec<u8>,
        expected: Option<RemoteTextMeta>,
        force: bool,
        backup: bool,
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
        mode: VerifyMode,
        algorithm: ChecksumAlgorithm,
    },
    /// 跨远端流式互传（单文件或整棵目录树）。
    RemoteTransfer {
        source_connection_id: ConnectionId,
        source_path: String,
        target_connection_id: ConnectionId,
        target_path: String,
        is_dir: bool,
        filter: TransferFilter,
        preserve_attributes: bool,
    },
    /// 远程归档（打包压缩）：将一组远端路径打包至目标归档文件。
    Archive {
        source_paths: Vec<String>,
        dest_archive_path: String,
    },
    /// 远程解压：将指定归档文件解压到目标目录。
    Extract {
        archive_path: String,
        dest_dir: String,
    },
}

impl SftpOperation {
    /// 判断当前 SFTP 操作是否属于大吞吐或长时间运行的数据传输类任务。
    ///
    /// 设计原理（Why）：
    /// - 实现初衷：SFTP 下载、上传与同步操作具有高带宽占用与深预取排队特征。若与目录列表（`List`）、
    ///   属性查询（`Stat`）等轻量交互操作共享单个 SFTP 会话通道，交互请求会被压在长队尾部产生明显的
    ///   长时间“加载中”卡顿；
    /// - 核心优势：通过语义准确划分传输任务与交互操作，驱动调度器复用或建立专用的后台传输通道，
    ///   使前台目录切换与浏览完全不受文件传输流水线影响；
    /// - 代价与局限：首次启动传输任务时需要额外建立一条 SSH 会话子通道（通常几十毫秒完成），
    ///   并在服务器端多启动一个独立的 `sftp-server` 子进程。
    #[must_use]
    pub fn is_transfer(&self) -> bool {
        matches!(
            self,
            Self::Download { .. }
                | Self::Upload { .. }
                | Self::DownloadDir { .. }
                | Self::UploadDir { .. }
                | Self::CompareDirs { .. }
                | Self::SyncDirs { .. }
                | Self::RemoteTransfer { .. }
        )
    }
}
