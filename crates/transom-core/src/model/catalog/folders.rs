//! 会话目录中的多层级文件夹增删改查、防环检测与默认属性继承。

use super::{CatalogError, FolderInheritedDefaults, SessionCatalog, SessionFolder};
use crate::ids::{FolderId, SessionId};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

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
}
