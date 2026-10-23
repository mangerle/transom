//! 会话目录：多层级文件夹与所属会话的聚合模型。
//!
//! 设计说明（Why）：
//! - 文件夹使用稳定 `FolderId` 与显式父指针，而非在会话上保存 `"A/B"` 路径字符串。
//!   这样重命名或移动文件夹只需改一条记录，不会出现同一文件夹被拆成多份的漂移；
//!   后续“文件夹继承默认属性”也能直接挂到稳定节点上。
//! - 代价是存储需要同时保存文件夹表，并在加载旧版 `group` 字符串时执行一次迁移。
//!   迁移集中在 `transom-store`，领域层始终只看到合法的树结构。

use crate::ids::{FolderId, SessionId};
use crate::session::Session;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// 会话文件夹节点。
///
/// `collapsed` 属于用户界面状态，但与文件夹一起持久化，保证重启后树形收纳状态不变。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionFolder {
    pub id: FolderId,
    pub name: String,
    #[serde(default)]
    pub parent: Option<FolderId>,
    #[serde(default)]
    pub collapsed: bool,
}

impl SessionFolder {
    pub fn new(name: impl Into<String>, parent: Option<FolderId>) -> Self {
        Self {
            id: FolderId::new(),
            name: name.into(),
            parent,
            collapsed: false,
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
}

/// 备份合并统计。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CatalogMergeSummary {
    pub added_sessions: usize,
    pub updated_sessions: usize,
    pub added_folders: usize,
    pub updated_folders: usize,
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

    /// 将文件夹移动到新的父文件夹（`None` 表示移到根层级）。
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
        let folder = self
            .folders
            .iter_mut()
            .find(|folder| folder.id == id)
            .ok_or(CatalogError::FolderNotFound(id))?;
        folder.parent = parent;
        Ok(())
    }

    /// 将会话移动到指定文件夹（`None` 表示移到根层级）。
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
        let session = self
            .sessions
            .iter_mut()
            .find(|session| session.id == session_id)
            .ok_or(CatalogError::SessionNotFound(session_id))?;
        session.folder_id = folder;
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
        self.normalize();
        summary
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
        });
        catalog.folders.push(SessionFolder {
            id: b,
            name: "B".into(),
            parent: Some(a),
            collapsed: false,
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
        });
        catalog.normalize();
        assert_eq!(catalog.folders.len(), 1);
        assert_eq!(catalog.folders[0].name, "one");
    }
}
