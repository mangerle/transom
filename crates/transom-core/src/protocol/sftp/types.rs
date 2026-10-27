//! SFTP 远端条目、元数据快照与文本指纹模型。

use serde::{Deserialize, Serialize};

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

/// 远端文本的版本指纹与形态标记：编辑面板打开或上次回传成功时记录。
///
/// 版本指纹部分（size/mtime）用于保存前与远端当前值比对，检测"打开后文件被
/// 他人修改"的并发冲突（丢失更新）。
///
/// 设计原理（Why）：stat-then-write 是乐观锁而非原子操作，比对与写入之间存在
/// 窗口；但它是 SFTP 协议下无需服务端配合的唯一可行方案，能拦住绝大多数
/// "打开到保存间隔数分钟"的真实冲突。代价与局限：mtime 精度为秒，同秒内同尺寸
/// 的修改无法识别；个别服务器不回传 mtime 时退化为仅按字节数比对。
///
/// 形态标记部分（bom/crlf/transcoded）描述读取时对原始字节做过的规整：编辑
/// 缓冲区持有的并非远端原样字节，回传时必须按标记还原，否则保存一次就把
/// BOM、CRLF 行尾悄悄改掉。transcoded 为 true 表示内容按 GBK 解码而来，
/// 只读预览、禁止回传。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RemoteTextMeta {
    /// 打开时或上次回传成功后的远端字节数（原始字节，含 BOM 与 CRLF）。
    ///
    /// 指纹比对的对象是远端 stat，必须用原始尺寸；编辑缓冲区长度因规整而
    /// 不同，不能混用。
    pub size: u64,
    /// 远端 mtime（秒）。`None` 表示服务器未回传，该维度视为通配不做比对，
    /// 避免偶发缺字段把合法保存误判成冲突。
    pub mtime: Option<u32>,
    /// 远端文件带 UTF-8 BOM，读取时已剥离，回传时需原样补回。
    pub bom: bool,
    /// 远端文件使用 CRLF 行尾，读取时已统一为 LF，回传时需原样还原。
    pub crlf: bool,
    /// 内容按 GBK 解码而来（原字节不是合法 UTF-8）；只读预览，禁止回传。
    pub transcoded: bool,
}

impl RemoteTextMeta {
    /// 与远端当前指纹不一致即视为冲突。
    ///
    /// 任一侧 mtime 缺失时跳过该维度，仅按字节数判定；形态标记不参与比对。
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
