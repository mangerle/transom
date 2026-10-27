//! 会话目录：多层级文件夹与所属会话的聚合模型。
//!
//! 设计说明（Why）：
//! - 文件夹使用稳定 `FolderId` 与显式父指针，而非在会话上保存 `"A/B"` 路径字符串。
//!   这样重命名或移动文件夹只需改一条记录，不会出现同一文件夹被拆成多份的漂移；
//!   后续“文件夹继承默认属性”也能直接挂到稳定节点上。
//! - 代价是存储需要同时保存文件夹表，并在加载旧版 `group` 字符串时执行一次迁移。
//!   迁移集中在 `transom-store`，领域层始终只看到合法的树结构。
//! - `folders` 与 `sessions` 两个数组的**原始顺序即同层级内的显示顺序**，
//!   所以拖拽排序只需重排数组，不引入额外序号字段：省掉一套“序号分配与压缩”逻辑，
//!   也避免序号重复导致的排序不稳定。代价是备份合并时新条目一律追加到末尾，
//!   以及任何写入都必须显式保持数组顺序（不能依赖按名称字典序渲染）。

mod folders;
mod reorder;
mod sessions;

#[cfg(test)]
mod tests;

use crate::ids::{FolderId, SessionId};
use crate::session::Session;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// ProxyJump 链路的最大嵌套层数；超出即视为配置异常而非合法拓扑。
pub const MAX_PROXY_JUMP_DEPTH: usize = 8;

/// 文件夹配置的默认继承属性。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FolderInheritedDefaults {
    pub port: Option<u16>,
    pub username: Option<String>,
    pub key_path: Option<PathBuf>,
}

/// 会话文件夹节点。
///
/// `collapsed` 属于用户界面状态，但与文件夹一起持久化，保证重启后树形收纳状态不变。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SessionFolder {
    pub id: FolderId,
    pub name: String,
    #[serde(default)]
    pub parent: Option<FolderId>,
    #[serde(default)]
    pub collapsed: bool,
    /// 默认 SSH 端口（下属会话未显式修改时自动继承）。
    #[serde(default)]
    pub default_port: Option<u16>,
    /// 默认用户名（下属会话未指定时自动继承）。
    #[serde(default)]
    pub default_username: Option<String>,
    /// 默认私钥路径（下属会话未配置时自动继承）。
    #[serde(default)]
    pub default_key_path: Option<PathBuf>,
}

impl SessionFolder {
    pub fn new(name: impl Into<String>, parent: Option<FolderId>) -> Self {
        Self {
            id: FolderId::new(),
            name: name.into(),
            parent,
            collapsed: false,
            default_port: None,
            default_username: None,
            default_key_path: None,
        }
    }
}

/// 会话与文件夹的完整持久化快照。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionCatalog {
    #[serde(default)]
    pub folders: Vec<SessionFolder>,
    #[serde(default)]
    pub sessions: Vec<Session>,
    /// 凭据模板清单；老目录文件没有该字段时按空清单降级。
    #[serde(default)]
    pub identities: Vec<crate::identity::Identity>,
}

/// 备份合并统计。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CatalogMergeSummary {
    pub added_sessions: usize,
    pub updated_sessions: usize,
    pub added_folders: usize,
    pub updated_folders: usize,
    /// 新增/更新的凭据模板数；口令是否随之落盘由导出选项决定，这里只报目录变化。
    pub added_identities: usize,
    pub updated_identities: usize,
}

/// 拖拽落点的侧向：插到锚点条目之前或之后。
///
/// 用枚举而非布尔位，是为了让“放到上/下半区”这类界面语义在调用点直接可读，
/// 也避免调用方把 `true/false` 顺序传错。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropSide {
    Before,
    After,
}

/// 会话树操作失败原因。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CatalogError {
    #[error("文件夹名称不能为空")]
    EmptyFolderName,
    #[error("文件夹名称不能包含斜杠 /")]
    InvalidFolderName,
    #[error("会话文件夹不存在: {0}")]
    FolderNotFound(FolderId),
    #[error("会话不存在: {0}")]
    SessionNotFound(SessionId),
    #[error("凭据模板不存在: {0}")]
    IdentityNotFound(crate::ids::IdentityId),
    #[error("凭据模板仍被 {0} 个会话引用，请先解除引用再删除")]
    IdentityInUse(usize),
    #[error("不能将文件夹移动到自身或其子文件夹: {0}")]
    FolderCycle(FolderId),
}
