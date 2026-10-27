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

use crate::error::{AppError, ErrorCode};
use crate::ids::{FolderId, SessionId};
use crate::session::{Connection, Session};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
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

impl SessionCatalog {
    /// 按 ID 查找文件夹。
    pub fn folder(&self, id: FolderId) -> Option<&SessionFolder> {
        self.folders.iter().find(|folder| folder.id == id)
    }

    /// 查找会话所属文件夹。
    pub fn folder_of_session(&self, session_id: SessionId) -> Option<&SessionFolder> {
        self.sessions
            .iter()
            .find(|session| session.id == session_id)
            .and_then(|session| session.folder_id)
            .and_then(|id| self.folder(id))
    }

    /// 返回从根到当前文件夹的名称链。
    ///
    /// 遇到损坏数据中的父子环时立即截断，避免界面渲染陷入死循环。
    pub fn folder_names(&self, id: FolderId) -> Vec<String> {
        let mut names = Vec::new();
        let by_id: HashMap<FolderId, &SessionFolder> = self
            .folders
            .iter()
            .map(|folder| (folder.id, folder))
            .collect();
        let mut visited = HashSet::new();
        let mut current = Some(id);
        while let Some(folder_id) = current {
            if !visited.insert(folder_id) {
                break;
            }
            let Some(folder) = by_id.get(&folder_id) else {
                break;
            };
            names.push(folder.name.clone());
            current = folder.parent;
        }
        names.reverse();
        names
    }

    /// 文件夹的展示路径，例如 `生产环境 / 数据库`。
    pub fn folder_path(&self, id: FolderId) -> Option<String> {
        let names = self.folder_names(id);
        (!names.is_empty()).then(|| names.join(" / "))
    }

    /// 判断 `candidate` 是否位于 `ancestor` 的子树中（包含自身）。
    ///
    /// 用于拒绝把父文件夹拖进自己的子文件夹而造成环。
    pub fn is_descendant_of(&self, candidate: FolderId, ancestor: FolderId) -> bool {
        let by_id: HashMap<FolderId, &SessionFolder> = self
            .folders
            .iter()
            .map(|folder| (folder.id, folder))
            .collect();
        let mut visited = HashSet::new();
        let mut current = Some(candidate);
        while let Some(folder_id) = current {
            if folder_id == ancestor {
                return true;
            }
            if !visited.insert(folder_id) {
                return false;
            }
            current = by_id.get(&folder_id).and_then(|folder| folder.parent);
        }
        false
    }

    /// 在指定父文件夹下新建子文件夹。
    ///
    /// # Errors
    /// 名称为空或父文件夹不存在时返回相应错误。
    pub fn add_folder(
        &mut self,
        name: &str,
        parent: Option<FolderId>,
    ) -> Result<FolderId, CatalogError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(CatalogError::EmptyFolderName);
        }
        if name.contains('/') {
            return Err(CatalogError::InvalidFolderName);
        }
        if let Some(parent_id) = parent
            && self.folder(parent_id).is_none()
        {
            return Err(CatalogError::FolderNotFound(parent_id));
        }
        let folder = SessionFolder::new(name, parent);
        let id = folder.id;
        self.folders.push(folder);
        Ok(id)
    }

    /// 在指定父文件夹下新建子文件夹，可指定默认继承属性。
    ///
    /// # Errors
    /// 名称为空或父文件夹不存在时返回相应错误。
    pub fn add_folder_with_defaults(
        &mut self,
        name: &str,
        parent: Option<FolderId>,
        default_port: Option<u16>,
        default_username: Option<String>,
        default_key_path: Option<PathBuf>,
    ) -> Result<FolderId, CatalogError> {
        let id = self.add_folder(name, parent)?;
        if default_port.is_some() || default_username.is_some() || default_key_path.is_some() {
            let _ = self.update_folder(id, name, default_port, default_username, default_key_path);
        }
        Ok(id)
    }

    /// 重命名文件夹。
    ///
    /// # Errors
    /// 名称为空或文件夹不存在时返回相应错误。
    pub fn rename_folder(&mut self, id: FolderId, name: &str) -> Result<(), CatalogError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(CatalogError::EmptyFolderName);
        }
        if name.contains('/') {
            return Err(CatalogError::InvalidFolderName);
        }
        let folder = self
            .folders
            .iter_mut()
            .find(|folder| folder.id == id)
            .ok_or(CatalogError::FolderNotFound(id))?;
        folder.name = name.to_string();
        Ok(())
    }

    /// 更新文件夹名称与默认继承属性。
    ///
    /// # Errors
    /// 名称为空或文件夹不存在时返回相应错误。
    pub fn update_folder(
        &mut self,
        id: FolderId,
        name: &str,
        default_port: Option<u16>,
        default_username: Option<String>,
        default_key_path: Option<PathBuf>,
    ) -> Result<(), CatalogError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(CatalogError::EmptyFolderName);
        }
        if name.contains('/') {
            return Err(CatalogError::InvalidFolderName);
        }
        let folder = self
            .folders
            .iter_mut()
            .find(|folder| folder.id == id)
            .ok_or(CatalogError::FolderNotFound(id))?;
        folder.name = name.to_string();
        folder.default_port = default_port;
        folder.default_username = default_username.filter(|u| !u.trim().is_empty());
        folder.default_key_path = default_key_path.filter(|p| !p.as_os_str().is_empty());
        Ok(())
    }

    /// 切换文件夹折叠状态。
    ///
    /// # Errors
    /// 文件夹不存在时返回错误。
    pub fn set_folder_collapsed(
        &mut self,
        id: FolderId,
        collapsed: bool,
    ) -> Result<(), CatalogError> {
        let folder = self
            .folders
            .iter_mut()
            .find(|folder| folder.id == id)
            .ok_or(CatalogError::FolderNotFound(id))?;
        folder.collapsed = collapsed;
        Ok(())
    }

    /// 将文件夹移动到新的父文件夹（`None` 表示移到根层级），并排到该层级末尾。
    ///
    /// 数组顺序即显示顺序，因此“放入容器”必须同时把节点搬到数组末尾，
    /// 否则被移动的文件夹会保留原下标，在新层级里落在意想不到的位置。
    ///
    /// # Errors
    /// 目标不存在，或目标位于自身子树中时返回错误。
    pub fn move_folder(
        &mut self,
        id: FolderId,
        parent: Option<FolderId>,
    ) -> Result<(), CatalogError> {
        if self.folder(id).is_none() {
            return Err(CatalogError::FolderNotFound(id));
        }
        if let Some(parent_id) = parent {
            if self.folder(parent_id).is_none() {
                return Err(CatalogError::FolderNotFound(parent_id));
            }
            if self.is_descendant_of(parent_id, id) {
                return Err(CatalogError::FolderCycle(id));
            }
        }
        let index = self
            .folders
            .iter()
            .position(|folder| folder.id == id)
            .ok_or(CatalogError::FolderNotFound(id))?;
        let mut folder = self.folders.remove(index);
        folder.parent = parent;
        self.folders.push(folder);
        Ok(())
    }

    /// 将会话移动到指定文件夹（`None` 表示移到根层级），并排到该层级末尾。
    ///
    /// # Errors
    /// 会话或目标文件夹不存在时返回错误。
    pub fn move_session(
        &mut self,
        session_id: SessionId,
        folder: Option<FolderId>,
    ) -> Result<(), CatalogError> {
        if let Some(folder_id) = folder
            && self.folder(folder_id).is_none()
        {
            return Err(CatalogError::FolderNotFound(folder_id));
        }
        let index = self
            .sessions
            .iter()
            .position(|session| session.id == session_id)
            .ok_or(CatalogError::SessionNotFound(session_id))?;
        let mut session = self.sessions.remove(index);
        session.folder_id = folder;
        self.sessions.push(session);
        Ok(())
    }

    /// 将会话移到锚点会话所在层级，并插到其前面或后面。
    ///
    /// 父级直接跟随锚点：拖到另一个分组里的会话上等于“移动 + 定位”一次完成，
    /// 用户不必先把条目移进目标分组再调顺序。锚点就是自身时视为无操作。
    ///
    /// # Errors
    /// 会话或锚点会话不存在时返回错误。
    pub fn reorder_session(
        &mut self,
        session_id: SessionId,
        anchor: SessionId,
        side: DropSide,
    ) -> Result<(), CatalogError> {
        if session_id == anchor {
            return Ok(());
        }
        let anchor_index = self
            .sessions
            .iter()
            .position(|session| session.id == anchor)
            .ok_or(CatalogError::SessionNotFound(anchor))?;
        let source_index = self
            .sessions
            .iter()
            .position(|session| session.id == session_id)
            .ok_or(CatalogError::SessionNotFound(session_id))?;
        let target_folder = self.sessions[anchor_index].folder_id;
        let mut session = self.sessions.remove(source_index);
        session.folder_id = target_folder;
        // 摘走自身会把后面元素左移一位，锚点下标需同步修正。
        let anchor_index = if source_index < anchor_index {
            anchor_index - 1
        } else {
            anchor_index
        };
        let insert_at = match side {
            DropSide::Before => anchor_index,
            DropSide::After => (anchor_index + 1).min(self.sessions.len()),
        };
        self.sessions.insert(insert_at, session);
        Ok(())
    }

    /// 将文件夹移到锚点文件夹所在层级，并插到其前面或后面。
    ///
    /// # Errors
    /// 文件夹或锚点不存在，或锚点位于被移动文件夹的子树内（会成环）时返回错误。
    pub fn reorder_folder(
        &mut self,
        folder_id: FolderId,
        anchor: FolderId,
        side: DropSide,
    ) -> Result<(), CatalogError> {
        if folder_id == anchor {
            return Ok(());
        }
        let anchor_index = self
            .folders
            .iter()
            .position(|folder| folder.id == anchor)
            .ok_or(CatalogError::FolderNotFound(anchor))?;
        let source_index = self
            .folders
            .iter()
            .position(|folder| folder.id == folder_id)
            .ok_or(CatalogError::FolderNotFound(folder_id))?;
        if self.is_descendant_of(anchor, folder_id) {
            return Err(CatalogError::FolderCycle(folder_id));
        }
        let target_parent = self.folders[anchor_index].parent;
        let mut folder = self.folders.remove(source_index);
        folder.parent = target_parent;
        let anchor_index = if source_index < anchor_index {
            anchor_index - 1
        } else {
            anchor_index
        };
        let insert_at = match side {
            DropSide::Before => anchor_index,
            DropSide::After => (anchor_index + 1).min(self.folders.len()),
        };
        self.folders.insert(insert_at, folder);
        Ok(())
    }

    /// 删除文件夹，但保留其中内容。
    ///
    /// 子文件夹与直属会话提升到被删除文件夹的父层级，避免误删用户资产；
    /// 返回被删除文件夹名称，供界面提示。
    ///
    /// # Errors
    /// 文件夹不存在时返回错误。
    pub fn delete_folder(&mut self, id: FolderId) -> Result<String, CatalogError> {
        let index = self
            .folders
            .iter()
            .position(|folder| folder.id == id)
            .ok_or(CatalogError::FolderNotFound(id))?;
        let removed = self.folders.remove(index);
        for folder in &mut self.folders {
            if folder.parent == Some(id) {
                folder.parent = removed.parent;
            }
        }
        for session in &mut self.sessions {
            if session.folder_id == Some(id) {
                session.folder_id = removed.parent;
            }
        }
        Ok(removed.name)
    }

    /// 按 `A/B/C` 路径查找或创建文件夹，返回末级文件夹。
    ///
    /// 供会话表单与旧版 `group` 字符串迁移使用；空路径返回 `None`。
    pub fn ensure_folder_path(&mut self, path: &str) -> Option<FolderId> {
        let mut parent = None;
        let mut resolved = None;
        for segment in path.split('/').map(str::trim).filter(|s| !s.is_empty()) {
            let existing = self
                .folders
                .iter()
                .find(|folder| folder.parent == parent && folder.name == segment)
                .map(|folder| folder.id);
            let id = match existing {
                Some(id) => id,
                None => self.add_folder(segment, parent).ok()?,
            };
            parent = Some(id);
            resolved = Some(id);
        }
        resolved
    }

    /// 合并另一份目录快照，用于备份导入与多机同步。
    ///
    /// 同 ID 的会话视为同一逻辑会话，导入方覆盖本地配置；同 ID 文件夹更新
    /// 名称与父级，但保留本机折叠状态（属于界面偏好）。合并后统一执行
    /// `normalize`，修复跨机器数据可能出现的悬空父级。
    pub fn merge(&mut self, other: SessionCatalog) -> CatalogMergeSummary {
        let mut summary = CatalogMergeSummary::default();
        for folder in other.folders {
            if let Some(existing) = self
                .folders
                .iter_mut()
                .find(|existing| existing.id == folder.id)
            {
                let collapsed = existing.collapsed;
                *existing = SessionFolder {
                    collapsed,
                    ..folder
                };
                summary.updated_folders += 1;
            } else {
                self.folders.push(folder);
                summary.added_folders += 1;
            }
        }
        for session in other.sessions {
            if let Some(existing) = self
                .sessions
                .iter_mut()
                .find(|existing| existing.id == session.id)
            {
                *existing = session;
                summary.updated_sessions += 1;
            } else {
                self.sessions.push(session);
                summary.added_sessions += 1;
            }
        }
        for identity in other.identities {
            if let Some(existing) = self
                .identities
                .iter_mut()
                .find(|existing| existing.id == identity.id)
            {
                *existing = identity;
                summary.updated_identities += 1;
            } else {
                self.identities.push(identity);
                summary.added_identities += 1;
            }
        }
        self.normalize();
        summary
    }

    /// 按 ID 查找凭据模板。
    pub fn identity(&self, id: crate::ids::IdentityId) -> Option<&crate::identity::Identity> {
        self.identities.iter().find(|identity| identity.id == id)
    }

    /// 仍引用该模板的会话数；删除前的闸门，也是界面提示的口径来源。
    pub fn identity_users(&self, id: crate::ids::IdentityId) -> usize {
        self.sessions
            .iter()
            .filter(|session| session.identity_id == Some(id))
            .count()
    }

    /// 删除凭据模板；仍被引用时拒绝并返回引用数。
    ///
    /// 为什么不做"级联清空引用"：删除模板后那些会话会静默退回自己的内联认证
    /// （往往是空的或过期的用户名），表现为"昨天还能连今天连不上"。让用户先
    /// 显式处理引用，比替他做决定安全。
    pub fn delete_identity(&mut self, id: crate::ids::IdentityId) -> Result<(), CatalogError> {
        if !self.identities.iter().any(|identity| identity.id == id) {
            return Err(CatalogError::IdentityNotFound(id));
        }
        let users = self.identity_users(id);
        if users > 0 {
            return Err(CatalogError::IdentityInUse(users));
        }
        self.identities.retain(|identity| identity.id != id);
        Ok(())
    }

    /// 计算指定文件夹沿祖先链路继承得到的默认属性（就近覆盖：子级未配置则向上查找）。
    pub fn inherited_folder_defaults(&self, folder_id: FolderId) -> FolderInheritedDefaults {
        let mut defaults = FolderInheritedDefaults::default();
        let by_id: HashMap<FolderId, &SessionFolder> = self
            .folders
            .iter()
            .map(|folder| (folder.id, folder))
            .collect();
        let mut visited = HashSet::new();
        let mut current = Some(folder_id);
        while let Some(id) = current {
            if !visited.insert(id) {
                break;
            }
            if let Some(folder) = by_id.get(&id) {
                if defaults.port.is_none() && folder.default_port.is_some() {
                    defaults.port = folder.default_port;
                }
                if defaults.username.is_none()
                    && folder
                        .default_username
                        .as_ref()
                        .is_some_and(|user| !user.trim().is_empty())
                {
                    defaults.username = folder.default_username.clone();
                }
                if defaults.key_path.is_none() && folder.default_key_path.is_some() {
                    defaults.key_path = folder.default_key_path.clone();
                }
                current = folder.parent;
            } else {
                break;
            }
        }
        defaults
    }

    /// 把文件夹继承属性与引用模板的用户名与认证方式叠加到会话副本上。
    ///
    /// 设计原理（Why）：叠加发生在连接派发前的最后一刻，net 层与 SFTP/转发等
    /// 所有下游只看到一个"已经知道自己该怎么登录"的 Session，不必到处判断
    /// "这个字段是会话的、文件夹继承的还是模板的"。
    /// 继承优先级：凭据模板 > 会话显式配置 > 文件夹链条继承。
    #[must_use]
    pub fn resolved_session(&self, session: &Session) -> Session {
        let mut resolved = session.clone();
        if let Some(folder_id) = session.folder_id {
            let defaults = self.inherited_folder_defaults(folder_id);
            if let crate::session::Connection::Ssh(ssh) = &mut resolved.connection {
                if ssh.port == 22
                    && let Some(port) = defaults.port
                {
                    ssh.port = port;
                }
                if ssh.username.trim().is_empty()
                    && let Some(username) = &defaults.username
                {
                    ssh.username = username.clone();
                }
                if let crate::session::AuthMethod::PrivateKey { path, .. } = &mut ssh.auth
                    && path.as_os_str().is_empty()
                    && let Some(key_path) = &defaults.key_path
                {
                    *path = key_path.clone();
                }
            }
        }
        let Some(Some(identity)) = session.identity_id.map(|id| self.identity(id)) else {
            return resolved;
        };
        if let crate::session::Connection::Ssh(ssh) = &mut resolved.connection {
            ssh.username = identity.username.clone();
            ssh.auth = identity.auth.clone();
        }
        resolved
    }

    /// 解析 ProxyJump 链路，返回从最外层跳板机到最内层跳板机的有序会话副本。
    ///
    /// 设计原理（Why）：链路的每一环都可能是"叠加快照"之后的会话（文件夹继承
    /// 端口/用户名、引用凭据模板），因此统一走 [`Self::resolved_session`] 产出
    /// 副本；调用方拿到的顺序即建连顺序——先连最外层跳板机，再逐级向内打洞。
    ///
    /// 环检测使用已访问集合而不是固定深度：自引用（会话把自己设为跳板机）与
    /// 任意长度的环都会在同一次遍历里被识别；另外设 `MAX_PROXY_JUMP_DEPTH`
    /// 上限，防止外部文件构造出超长链路拖垮连接流程。
    ///
    /// # Errors
    /// 引用的跳板会话不存在、指向非 SSH 会话、超过层数上限或存在环形依赖时，
    /// 返回 `Configuration` 错误并附带中文现场。
    pub fn proxy_jump_chain(&self, session: &Session) -> Result<Vec<Session>, AppError> {
        let mut chain = Vec::new();
        let Connection::Ssh(ssh) = &session.connection else {
            return Ok(chain);
        };
        let mut visited: HashSet<SessionId> = HashSet::with_capacity(4);
        visited.insert(session.id);
        let mut cursor = ssh.proxy_jump.as_ref().map(|jump| jump.jump_session_id);
        while let Some(id) = cursor {
            if !visited.insert(id) {
                return Err(AppError::with_detail(
                    ErrorCode::Configuration,
                    format!(
                        "跳板机链路存在环形依赖，会话: {}，重复引用: {id}",
                        session.name
                    ),
                ));
            }
            if chain.len() >= MAX_PROXY_JUMP_DEPTH {
                return Err(AppError::with_detail(
                    ErrorCode::Configuration,
                    format!(
                        "跳板机链路超过 {MAX_PROXY_JUMP_DEPTH} 层上限，会话: {}",
                        session.name
                    ),
                ));
            }
            let Some(hop) = self.sessions.iter().find(|candidate| candidate.id == id) else {
                return Err(AppError::with_detail(
                    ErrorCode::Configuration,
                    format!(
                        "引用的跳板机会话已不存在，会话: {}，跳板机: {id}",
                        session.name
                    ),
                ));
            };
            let Connection::Ssh(hop_ssh) = &hop.connection else {
                return Err(AppError::with_detail(
                    ErrorCode::Configuration,
                    format!(
                        "跳板机会话不是 SSH 连接，会话: {}，跳板机: {}",
                        session.name, hop.name
                    ),
                ));
            };
            chain.push(self.resolved_session(hop));
            cursor = hop_ssh.proxy_jump.as_ref().map(|jump| jump.jump_session_id);
        }
        chain.reverse();
        Ok(chain)
    }

    /// 修复外部文件中的损坏树结构，保证后续 UI 与移动操作安全。
    ///
    /// 处理内容：重复文件夹 ID、缺失/自引用父节点、父子环、指向不存在文件夹的会话。
    /// 这类数据可能来自手工编辑或跨版本导入，必须在进入内存时一次性收敛。
    pub fn normalize(&mut self) {
        let mut seen = HashSet::with_capacity(self.folders.len());
        self.folders.retain(|folder| seen.insert(folder.id));

        let ids: HashSet<FolderId> = self.folders.iter().map(|folder| folder.id).collect();
        for folder in &mut self.folders {
            if folder
                .parent
                .is_some_and(|parent| parent == folder.id || !ids.contains(&parent))
            {
                folder.parent = None;
            }
        }

        let parents: HashMap<FolderId, Option<FolderId>> = self
            .folders
            .iter()
            .map(|folder| (folder.id, folder.parent))
            .collect();
        let mut cycle_roots = Vec::new();
        for folder in &self.folders {
            let mut visited = HashSet::new();
            let mut current = Some(folder.id);
            while let Some(id) = current {
                if !visited.insert(id) {
                    cycle_roots.push(folder.id);
                    break;
                }
                current = parents.get(&id).copied().flatten();
            }
        }
        for id in cycle_roots {
            if let Some(folder) = self.folders.iter_mut().find(|folder| folder.id == id) {
                folder.parent = None;
            }
        }

        let folder_ids: HashSet<FolderId> = self.folders.iter().map(|folder| folder.id).collect();
        for session in &mut self.sessions {
            if session
                .folder_id
                .is_some_and(|id| !folder_ids.contains(&id))
            {
                session.folder_id = None;
            }
        }

        // 模板同理：重复 ID 只留第一个，悬空引用清成"未引用"，
        // 让会话退回自己的内联认证而不是带着坏指针去连接。
        let mut seen_identities = HashSet::with_capacity(self.identities.len());
        self.identities
            .retain(|identity| seen_identities.insert(identity.id));
        let identity_ids: HashSet<crate::ids::IdentityId> =
            self.identities.iter().map(|identity| identity.id).collect();
        for session in &mut self.sessions {
            if session
                .identity_id
                .is_some_and(|id| !identity_ids.contains(&id))
            {
                session.identity_id = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{Connection, LocalConnection};

    fn session(catalog: &mut SessionCatalog, name: &str) -> SessionId {
        let session = Session::new(name, Connection::Local(LocalConnection::default()));
        let id = session.id;
        catalog.sessions.push(session);
        id
    }

    fn session_ids(catalog: &SessionCatalog) -> Vec<SessionId> {
        catalog.sessions.iter().map(|session| session.id).collect()
    }

    fn folder_ids(catalog: &SessionCatalog) -> Vec<FolderId> {
        catalog.folders.iter().map(|folder| folder.id).collect()
    }

    #[test]
    fn ensure_path_creates_and_reuses_nested_folders() {
        let mut catalog = SessionCatalog::default();
        let leaf = catalog.ensure_folder_path("生产/数据库").unwrap();
        assert_eq!(catalog.folder_names(leaf), vec!["生产", "数据库"]);
        assert_eq!(catalog.ensure_folder_path("生产/数据库"), Some(leaf));
        assert_eq!(catalog.folders.len(), 2);
    }

    #[test]
    fn folder_names_reject_slashes() {
        let mut catalog = SessionCatalog::default();
        assert_eq!(
            catalog.add_folder("A/B", None),
            Err(CatalogError::InvalidFolderName)
        );
        let id = catalog.add_folder("A", None).unwrap();
        assert_eq!(
            catalog.rename_folder(id, "A/B"),
            Err(CatalogError::InvalidFolderName)
        );
    }

    #[test]
    fn move_folder_rejects_cycle() {
        let mut catalog = SessionCatalog::default();
        let parent = catalog.ensure_folder_path("A").unwrap();
        let child = catalog.ensure_folder_path("A/B/C").unwrap();
        assert_eq!(
            catalog.move_folder(parent, Some(child)),
            Err(CatalogError::FolderCycle(parent))
        );
        assert!(catalog.move_folder(child, Some(parent)).is_ok());
    }

    /// 数组顺序即显示顺序：向前/向后插入必须精确改变同级先后。
    #[test]
    fn reorder_session_places_relative_to_anchor() {
        let mut catalog = SessionCatalog::default();
        let folder = catalog.ensure_folder_path("A").unwrap();
        let first = session(&mut catalog, "first");
        let second = session(&mut catalog, "second");
        let third = session(&mut catalog, "third");
        for id in [first, second, third] {
            catalog.move_session(id, Some(folder)).unwrap();
        }
        assert_eq!(
            session_ids(&catalog),
            vec![first, second, third],
            "放入同一文件夹应按移动先后排列"
        );

        catalog
            .reorder_session(third, first, DropSide::After)
            .unwrap();
        assert_eq!(session_ids(&catalog), vec![first, third, second]);

        catalog
            .reorder_session(first, second, DropSide::After)
            .unwrap();
        assert_eq!(session_ids(&catalog), vec![third, second, first]);

        catalog
            .reorder_session(second, third, DropSide::Before)
            .unwrap();
        assert_eq!(session_ids(&catalog), vec![second, third, first]);
    }

    /// 跨分组拖到某条会话上时，父级跟随锚点，一次操作完成“移动 + 定位”。
    #[test]
    fn reorder_session_follows_anchor_folder() {
        let mut catalog = SessionCatalog::default();
        let left = catalog.ensure_folder_path("A").unwrap();
        let right = catalog.ensure_folder_path("B").unwrap();
        let alpha = session(&mut catalog, "alpha");
        let beta = session(&mut catalog, "beta");
        let gamma = session(&mut catalog, "gamma");
        catalog.move_session(alpha, Some(left)).unwrap();
        catalog.move_session(beta, Some(left)).unwrap();
        catalog.move_session(gamma, Some(right)).unwrap();

        catalog
            .reorder_session(gamma, beta, DropSide::After)
            .unwrap();

        assert_eq!(catalog.sessions[2].id, gamma);
        assert_eq!(catalog.sessions[2].folder_id, Some(left));
        assert_eq!(session_ids(&catalog), vec![alpha, beta, gamma]);
    }

    /// 锚点就是自身、或锚点不存在时，不得破坏既有顺序。
    #[test]
    fn reorder_session_rejects_invalid_anchor() {
        let mut catalog = SessionCatalog::default();
        let alpha = session(&mut catalog, "alpha");
        let beta = session(&mut catalog, "beta");

        catalog
            .reorder_session(alpha, alpha, DropSide::Before)
            .unwrap();
        assert_eq!(session_ids(&catalog), vec![alpha, beta]);

        let missing = SessionId::new();
        assert_eq!(
            catalog.reorder_session(alpha, missing, DropSide::Before),
            Err(CatalogError::SessionNotFound(missing))
        );
        assert_eq!(session_ids(&catalog), vec![alpha, beta]);
    }

    /// 文件夹重排同样只改同级先后，并且禁止把父级插到自己子级之间。
    #[test]
    fn reorder_folder_orders_siblings_and_rejects_cycle() {
        let mut catalog = SessionCatalog::default();
        let a = catalog.ensure_folder_path("A").unwrap();
        let b = catalog.ensure_folder_path("B").unwrap();
        let c = catalog.ensure_folder_path("C").unwrap();
        assert_eq!(folder_ids(&catalog), vec![a, b, c]);

        catalog.reorder_folder(c, a, DropSide::After).unwrap();
        assert_eq!(folder_ids(&catalog), vec![a, c, b]);

        let child_of_a = catalog.ensure_folder_path("A/D").unwrap();
        assert_eq!(
            catalog.reorder_folder(a, child_of_a, DropSide::Before),
            Err(CatalogError::FolderCycle(a))
        );

        // 锚点位于另一个层级时，父级跟随锚点。
        catalog
            .reorder_folder(b, child_of_a, DropSide::Before)
            .unwrap();
        assert_eq!(catalog.folder(b).unwrap().parent, Some(a));
        assert_eq!(folder_ids(&catalog), vec![a, c, b, child_of_a]);
    }

    #[test]
    fn delete_folder_promotes_contents_to_parent() {
        let mut catalog = SessionCatalog::default();
        let parent = catalog.ensure_folder_path("A").unwrap();
        let child = catalog.ensure_folder_path("A/B").unwrap();
        let session_id = session(&mut catalog, "db");
        catalog.move_session(session_id, Some(child)).unwrap();

        assert_eq!(catalog.delete_folder(child).unwrap(), "B");
        assert_eq!(catalog.folder(child), None);
        assert_eq!(
            catalog.sessions[0].folder_id,
            Some(parent),
            "会话应提升到被删文件夹的父级"
        );
    }

    #[test]
    fn normalize_breaks_cycles_and_missing_parents() {
        let mut catalog = SessionCatalog::default();
        let a = FolderId::new();
        let b = FolderId::new();
        catalog.folders.push(SessionFolder {
            id: a,
            name: "A".into(),
            parent: Some(b),
            collapsed: false,
            ..Default::default()
        });
        catalog.folders.push(SessionFolder {
            id: b,
            name: "B".into(),
            parent: Some(a),
            collapsed: false,
            ..Default::default()
        });
        let orphan = session(&mut catalog, "lost");
        catalog.sessions[0].folder_id = Some(FolderId::new());

        catalog.normalize();

        assert!(catalog.folder(a).unwrap().parent.is_none());
        assert!(catalog.folder(b).unwrap().parent.is_none());
        assert_eq!(catalog.sessions[0].id, orphan);
        assert_eq!(catalog.sessions[0].folder_id, None);
    }

    #[test]
    fn merge_adds_new_and_updates_same_ids() {
        let mut local = SessionCatalog::default();
        let folder = local.add_folder("本地", None).unwrap();
        local.set_folder_collapsed(folder, true).unwrap();
        let local_session = session(&mut local, "same");

        let mut remote = SessionCatalog::default();
        remote.folders.push(SessionFolder {
            id: folder,
            name: "远端".into(),
            parent: None,
            collapsed: false,
            ..Default::default()
        });
        let mut same = Session::new("same", Connection::Local(LocalConnection::default()));
        same.id = local_session;
        let new_session = Session::new("new", Connection::Local(LocalConnection::default()));
        remote.sessions = vec![same, new_session];

        let summary = local.merge(remote);
        assert_eq!(summary.added_sessions, 1);
        assert_eq!(summary.updated_sessions, 1);
        assert_eq!(summary.updated_folders, 1);
        assert_eq!(local.folders[0].name, "远端");
        assert!(local.folders[0].collapsed, "折叠状态应保留本机偏好");
        assert_eq!(local.sessions.len(), 2);
    }

    #[test]
    fn normalize_drops_duplicate_folder_ids() {
        let mut catalog = SessionCatalog::default();
        let id = FolderId::new();
        catalog.folders.push(SessionFolder::new("one", None));
        catalog.folders[0].id = id;
        catalog.folders.push(SessionFolder {
            id,
            name: "duplicate".into(),
            parent: None,
            collapsed: false,
            ..Default::default()
        });
        catalog.normalize();
        assert_eq!(catalog.folders.len(), 1);
        assert_eq!(catalog.folders[0].name, "one");
    }

    fn ssh_session(catalog: &mut SessionCatalog, name: &str) -> SessionId {
        let session = Session::new(
            name,
            crate::session::Connection::Ssh(crate::session::SshConnection::new("h", "inline-user")),
        );
        let id = session.id;
        catalog.sessions.push(session);
        id
    }

    fn ops(catalog: &mut SessionCatalog) -> crate::ids::IdentityId {
        let identity = crate::identity::Identity::new(
            "ops",
            "ops-user",
            crate::session::AuthMethod::Password { save: true },
        );
        let id = identity.id;
        catalog.identities.push(identity);
        id
    }

    fn set_jump(catalog: &mut SessionCatalog, session: SessionId, jump: SessionId) {
        let session = catalog
            .sessions
            .iter_mut()
            .find(|candidate| candidate.id == session)
            .unwrap();
        let crate::session::Connection::Ssh(ssh) = &mut session.connection else {
            panic!("测试会话必须是 SSH 连接");
        };
        ssh.proxy_jump = Some(crate::session::ProxyJumpConfig {
            jump_session_id: jump,
        });
    }

    /// 多层跳板链必须按"最外层 -> 最内层"返回，建连方按此顺序逐级打洞。
    #[test]
    fn proxy_jump_chain_is_ordered_outermost_first() {
        let mut catalog = SessionCatalog::default();
        let outer = ssh_session(&mut catalog, "outer");
        let inner = ssh_session(&mut catalog, "inner");
        let target = ssh_session(&mut catalog, "target");
        set_jump(&mut catalog, inner, outer);
        set_jump(&mut catalog, target, inner);

        let session = catalog
            .sessions
            .iter()
            .find(|candidate| candidate.id == target)
            .unwrap();
        let chain = catalog.proxy_jump_chain(session).unwrap();
        let names: Vec<&str> = chain.iter().map(|hop| hop.name.as_str()).collect();
        assert_eq!(names, vec!["outer", "inner"]);
        // 链路副本同样叠加了模板/文件夹继承，与直连目标保持同一口径。
        assert!(matches!(
            chain[0].connection,
            crate::session::Connection::Ssh(_)
        ));
    }

    #[test]
    fn direct_connection_has_empty_proxy_chain() {
        let mut catalog = SessionCatalog::default();
        let id = ssh_session(&mut catalog, "direct");
        let session = catalog
            .sessions
            .iter()
            .find(|candidate| candidate.id == id)
            .unwrap();
        assert!(catalog.proxy_jump_chain(session).unwrap().is_empty());
    }

    #[test]
    fn proxy_jump_chain_rejects_self_cycle_and_dangling_references() {
        let mut catalog = SessionCatalog::default();
        let lone = ssh_session(&mut catalog, "lone");
        set_jump(&mut catalog, lone, lone);
        let session = catalog
            .sessions
            .iter()
            .find(|candidate| candidate.id == lone)
            .unwrap();
        let error = catalog.proxy_jump_chain(session).unwrap_err();
        assert_eq!(error.code, ErrorCode::Configuration);
        assert!(error.detail.as_deref().unwrap_or_default().contains("环形"));

        let mut catalog = SessionCatalog::default();
        let first = ssh_session(&mut catalog, "first");
        let second = ssh_session(&mut catalog, "second");
        set_jump(&mut catalog, first, second);
        set_jump(&mut catalog, second, first);
        let session = catalog
            .sessions
            .iter()
            .find(|candidate| candidate.id == first)
            .unwrap();
        assert!(catalog.proxy_jump_chain(session).is_err());

        let mut catalog = SessionCatalog::default();
        let dangling = ssh_session(&mut catalog, "dangling");
        set_jump(&mut catalog, dangling, SessionId::new());
        let session = catalog
            .sessions
            .iter()
            .find(|candidate| candidate.id == dangling)
            .unwrap();
        let error = catalog.proxy_jump_chain(session).unwrap_err();
        assert!(
            error
                .detail
                .as_deref()
                .unwrap_or_default()
                .contains("不存在")
        );
    }

    /// 跳板机只能是 SSH 会话：串口/本地终端没有可复用的加密隧道。
    #[test]
    fn proxy_jump_chain_rejects_non_ssh_jump() {
        let mut catalog = SessionCatalog::default();
        let local = Session::new(
            "local",
            crate::session::Connection::Local(crate::session::LocalConnection::default()),
        );
        let local_id = local.id;
        catalog.sessions.push(local);
        let target = ssh_session(&mut catalog, "target");
        set_jump(&mut catalog, target, local_id);

        let session = catalog
            .sessions
            .iter()
            .find(|candidate| candidate.id == target)
            .unwrap();
        let error = catalog.proxy_jump_chain(session).unwrap_err();
        assert!(
            error
                .detail
                .as_deref()
                .unwrap_or_default()
                .contains("不是 SSH")
        );
    }

    /// 模板叠加只改身份（用户名 + 认证方式），主机、端口与机器级选项保持不变。
    #[test]
    fn resolved_session_overrides_identity_fields_only() {
        let mut catalog = SessionCatalog::default();
        let identity_id = ops(&mut catalog);
        let session_id = ssh_session(&mut catalog, "web");
        catalog
            .sessions
            .iter_mut()
            .find(|session| session.id == session_id)
            .unwrap()
            .identity_id = Some(identity_id);

        let session = catalog
            .sessions
            .iter()
            .find(|session| session.id == session_id)
            .unwrap();
        let resolved = catalog.resolved_session(session);
        match &resolved.connection {
            crate::session::Connection::Ssh(ssh) => {
                assert_eq!(ssh.username, "ops-user");
                assert_eq!(ssh.host, "h");
                assert_eq!(ssh.port, 22);
                assert!(matches!(
                    ssh.auth,
                    crate::session::AuthMethod::Password { save: true }
                ));
            }
            _ => panic!("应仍是 SSH 连接"),
        }
        // 原会话不被修改：叠加发生在连接派发时的副本上。
        match &session.connection {
            crate::session::Connection::Ssh(ssh) => assert_eq!(ssh.username, "inline-user"),
            _ => panic!("应仍是 SSH 连接"),
        }
    }

    /// 未引用模板时逐字段原样返回，保证老会话行为不变。
    #[test]
    fn resolved_session_passes_through_without_reference() {
        let catalog = SessionCatalog::default();
        let session = Session::new(
            "web",
            crate::session::Connection::Ssh(crate::session::SshConnection::new("h", "u")),
        );
        let resolved = catalog.resolved_session(&session);
        match &resolved.connection {
            crate::session::Connection::Ssh(ssh) => {
                assert_eq!(ssh.username, "u");
                assert!(matches!(ssh.auth, crate::session::AuthMethod::Agent));
            }
            _ => panic!("应仍是 SSH 连接"),
        }
        assert_eq!(resolved.id, session.id);
    }

    /// 引用模板的会话必须先解除引用才能删模板：静默清引用会让那些会话
    /// 突然退回（可能为空的）内联认证，表现为"昨天还能连今天连不上"。
    #[test]
    fn delete_identity_refuses_while_referenced() {
        let mut catalog = SessionCatalog::default();
        let identity_id = ops(&mut catalog);
        let session_id = ssh_session(&mut catalog, "web");
        catalog
            .sessions
            .iter_mut()
            .find(|session| session.id == session_id)
            .unwrap()
            .identity_id = Some(identity_id);

        assert_eq!(catalog.identity_users(identity_id), 1);
        assert_eq!(
            catalog.delete_identity(identity_id),
            Err(CatalogError::IdentityInUse(1))
        );
        assert_eq!(catalog.identities.len(), 1);

        catalog
            .sessions
            .iter_mut()
            .find(|session| session.id == session_id)
            .unwrap()
            .identity_id = None;
        assert_eq!(catalog.identity_users(identity_id), 0);
        assert!(catalog.delete_identity(identity_id).is_ok());
        assert!(catalog.identities.is_empty());
        assert!(matches!(
            catalog.delete_identity(identity_id),
            Err(CatalogError::IdentityNotFound(_))
        ));
    }

    /// 悬空模板引用与重复模板 id 都要在归一时收敛：坏数据不能让会话带着
    /// 无效指针去连接，也不能让口令存进两个同 id 槽位。
    #[test]
    fn normalize_clears_dangling_identity_references() {
        let mut catalog = SessionCatalog::default();
        let identity_id = ops(&mut catalog);
        let session_id = ssh_session(&mut catalog, "web");
        catalog
            .sessions
            .iter_mut()
            .find(|session| session.id == session_id)
            .unwrap()
            .identity_id = Some(crate::ids::IdentityId::new());
        let duplicate = catalog.identities[0].clone();
        catalog.identities.push(duplicate);

        catalog.normalize();
        assert_eq!(catalog.identities.len(), 1);
        assert_eq!(catalog.identities[0].id, identity_id);
        assert!(catalog.sessions[0].identity_id.is_none());
    }

    #[test]
    fn folder_defaults_inheritance_and_session_resolution() {
        let mut catalog = SessionCatalog::default();
        let root = catalog.add_folder("root", None).unwrap();
        catalog
            .update_folder(
                root,
                "root",
                Some(2200),
                Some("root_user".to_string()),
                Some(PathBuf::from("/keys/root_id_rsa")),
            )
            .unwrap();

        let sub = catalog.add_folder("sub", Some(root)).unwrap();
        // sub 只覆盖端口，用户名和私钥留空，继承自 root
        catalog
            .update_folder(sub, "sub", Some(2222), None, None)
            .unwrap();

        let defaults = catalog.inherited_folder_defaults(sub);
        assert_eq!(defaults.port, Some(2222));
        assert_eq!(defaults.username.as_deref(), Some("root_user"));
        assert_eq!(
            defaults.key_path.as_deref(),
            Some(std::path::Path::new("/keys/root_id_rsa"))
        );

        // 新建一个属于 sub 的会话，用户名为空，端口为默认 22，私钥为空
        let mut session = Session::new(
            "child_session",
            crate::session::Connection::Ssh(crate::session::SshConnection {
                host: "server.internal".into(),
                port: 22,
                username: "".into(),
                auth: crate::session::AuthMethod::PrivateKey {
                    path: PathBuf::new(),
                    cert_path: None,
                    save_passphrase: false,
                },
                options: crate::session::SessionOptions::default(),
                proxy_jump: None,
            }),
        );
        session.folder_id = Some(sub);

        let resolved = catalog.resolved_session(&session);
        if let crate::session::Connection::Ssh(ssh) = resolved.connection {
            assert_eq!(ssh.port, 2222);
            assert_eq!(ssh.username, "root_user");
            if let crate::session::AuthMethod::PrivateKey { path, .. } = ssh.auth {
                assert_eq!(path, PathBuf::from("/keys/root_id_rsa"));
            } else {
                panic!("应为私钥认证");
            }
        } else {
            panic!("应为 SSH 连接");
        }
    }
}
