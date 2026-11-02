//! 引擎连接分组树的增删改查、防环与拖拽排序。
//!
//! 设计原理（Why）：与会话文件夹保持同一套稳定 ID + 父指针模型，顺序由数组
//! 位置决定；这样备份合并、拖拽排序与界面渲染都能复用既有心智，不引入序号字段。
//! 代价是与 `folders.rs` 有两份结构相似但互不继承的代码，换取两类资源互不牵连。

use super::{CatalogError, DropSide, EngineFolder, SessionCatalog};
use crate::ids::{EngineProfileId, FolderId};
use std::collections::{HashMap, HashSet};

impl SessionCatalog {
    /// 按 ID 查找引擎分组。
    pub fn engine_folder(&self, id: FolderId) -> Option<&EngineFolder> {
        self.engine_folders.iter().find(|folder| folder.id == id)
    }

    /// 查找引擎连接所属分组。
    pub fn engine_folder_of(&self, profile_id: EngineProfileId) -> Option<&EngineFolder> {
        self.engines
            .iter()
            .find(|profile| profile.id == profile_id)
            .and_then(|profile| profile.folder_id)
            .and_then(|id| self.engine_folder(id))
    }

    /// 从根到当前分组的名称链；遇到损坏环时立即截断。
    pub fn engine_folder_names(&self, id: FolderId) -> Vec<String> {
        let mut names = Vec::new();
        let by_id: HashMap<FolderId, &EngineFolder> = self
            .engine_folders
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

    /// 分组的展示路径，例如 `生产 / 数据库`。
    pub fn engine_folder_path(&self, id: FolderId) -> Option<String> {
        let names = self.engine_folder_names(id);
        (!names.is_empty()).then(|| names.join(" / "))
    }

    /// 判断 `candidate` 是否位于 `ancestor` 的子树中（包含自身）。
    pub fn is_engine_folder_descendant(&self, candidate: FolderId, ancestor: FolderId) -> bool {
        let by_id: HashMap<FolderId, &EngineFolder> = self
            .engine_folders
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

    /// 在指定父分组下新建子分组。
    ///
    /// # Errors
    /// 名称为空、含斜杠或父分组不存在时返回错误。
    pub fn add_engine_folder(
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
            && self.engine_folder(parent_id).is_none()
        {
            return Err(CatalogError::EngineFolderNotFound(parent_id));
        }
        let folder = EngineFolder::new(name, parent);
        let id = folder.id;
        self.engine_folders.push(folder);
        Ok(id)
    }

    /// 重命名引擎分组。
    ///
    /// # Errors
    /// 名称为空、含斜杠或分组不存在时返回错误。
    pub fn rename_engine_folder(&mut self, id: FolderId, name: &str) -> Result<(), CatalogError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(CatalogError::EmptyFolderName);
        }
        if name.contains('/') {
            return Err(CatalogError::InvalidFolderName);
        }
        let folder = self
            .engine_folders
            .iter_mut()
            .find(|folder| folder.id == id)
            .ok_or(CatalogError::EngineFolderNotFound(id))?;
        folder.name = name.to_string();
        Ok(())
    }

    /// 切换引擎分组的折叠状态。
    ///
    /// # Errors
    /// 分组不存在时返回错误。
    pub fn set_engine_folder_collapsed(
        &mut self,
        id: FolderId,
        collapsed: bool,
    ) -> Result<(), CatalogError> {
        let folder = self
            .engine_folders
            .iter_mut()
            .find(|folder| folder.id == id)
            .ok_or(CatalogError::EngineFolderNotFound(id))?;
        folder.collapsed = collapsed;
        Ok(())
    }

    /// 把分组移到新的父分组末尾（`None` 表示根层级）。
    ///
    /// # Errors
    /// 分组不存在、目标不存在或目标位于自身子树中（会成环）时返回错误。
    pub fn move_engine_folder(
        &mut self,
        id: FolderId,
        parent: Option<FolderId>,
    ) -> Result<(), CatalogError> {
        if self.engine_folder(id).is_none() {
            return Err(CatalogError::EngineFolderNotFound(id));
        }
        if let Some(parent_id) = parent {
            if self.engine_folder(parent_id).is_none() {
                return Err(CatalogError::EngineFolderNotFound(parent_id));
            }
            if self.is_engine_folder_descendant(parent_id, id) {
                return Err(CatalogError::EngineFolderCycle(id));
            }
        }
        let index = self
            .engine_folders
            .iter()
            .position(|folder| folder.id == id)
            .ok_or(CatalogError::EngineFolderNotFound(id))?;
        let mut folder = self.engine_folders.remove(index);
        folder.parent = parent;
        self.engine_folders.push(folder);
        Ok(())
    }

    /// 删除分组但保留内容：子分组与直属引擎提升到被删分组的父层级。
    ///
    /// # Errors
    /// 分组不存在时返回错误。
    pub fn delete_engine_folder(&mut self, id: FolderId) -> Result<String, CatalogError> {
        let index = self
            .engine_folders
            .iter()
            .position(|folder| folder.id == id)
            .ok_or(CatalogError::EngineFolderNotFound(id))?;
        let removed = self.engine_folders.remove(index);
        for folder in &mut self.engine_folders {
            if folder.parent == Some(id) {
                folder.parent = removed.parent;
            }
        }
        for profile in &mut self.engines {
            if profile.folder_id == Some(id) {
                profile.folder_id = removed.parent;
                profile.touch();
            }
        }
        Ok(removed.name)
    }

    /// 把引擎连接放进指定分组（`None` 表示根层级），并排到该层级末尾。
    ///
    /// # Errors
    /// 分组不存在或引擎连接不存在时返回错误。
    pub fn move_engine(
        &mut self,
        profile_id: EngineProfileId,
        folder: Option<FolderId>,
    ) -> Result<(), CatalogError> {
        if let Some(folder_id) = folder
            && self.engine_folder(folder_id).is_none()
        {
            return Err(CatalogError::EngineFolderNotFound(folder_id));
        }
        let index = self
            .engines
            .iter()
            .position(|profile| profile.id == profile_id)
            .ok_or(CatalogError::EngineNotFound(profile_id))?;
        let mut profile = self.engines.remove(index);
        profile.folder_id = folder;
        profile.touch();
        self.engines.push(profile);
        Ok(())
    }

    /// 把引擎连接移到锚点连接所在层级，并插到其前/后。
    ///
    /// # Errors
    /// 引擎连接或锚点不存在时返回错误。
    pub fn reorder_engine(
        &mut self,
        profile_id: EngineProfileId,
        anchor: EngineProfileId,
        side: DropSide,
    ) -> Result<(), CatalogError> {
        if profile_id == anchor {
            return Ok(());
        }
        let anchor_index = self
            .engines
            .iter()
            .position(|profile| profile.id == anchor)
            .ok_or(CatalogError::EngineNotFound(anchor))?;
        let source_index = self
            .engines
            .iter()
            .position(|profile| profile.id == profile_id)
            .ok_or(CatalogError::EngineNotFound(profile_id))?;
        let target_folder = self.engines[anchor_index].folder_id;
        let mut profile = self.engines.remove(source_index);
        profile.folder_id = target_folder;
        profile.touch();
        let anchor_index = if source_index < anchor_index {
            anchor_index - 1
        } else {
            anchor_index
        };
        let insert_at = match side {
            DropSide::Before => anchor_index,
            DropSide::After => (anchor_index + 1).min(self.engines.len()),
        };
        self.engines.insert(insert_at, profile);
        Ok(())
    }

    /// 把分组移到锚点分组所在层级，并插到其前/后。
    ///
    /// # Errors
    /// 分组或锚点不存在，或锚点位于被移动分组的子树内（会成环）时返回错误。
    pub fn reorder_engine_folder(
        &mut self,
        folder_id: FolderId,
        anchor: FolderId,
        side: DropSide,
    ) -> Result<(), CatalogError> {
        if folder_id == anchor {
            return Ok(());
        }
        let anchor_index = self
            .engine_folders
            .iter()
            .position(|folder| folder.id == anchor)
            .ok_or(CatalogError::EngineFolderNotFound(anchor))?;
        let source_index = self
            .engine_folders
            .iter()
            .position(|folder| folder.id == folder_id)
            .ok_or(CatalogError::EngineFolderNotFound(folder_id))?;
        if self.is_engine_folder_descendant(anchor, folder_id) {
            return Err(CatalogError::EngineFolderCycle(folder_id));
        }
        let target_parent = self.engine_folders[anchor_index].parent;
        let mut folder = self.engine_folders.remove(source_index);
        folder.parent = target_parent;
        let anchor_index = if source_index < anchor_index {
            anchor_index - 1
        } else {
            anchor_index
        };
        let insert_at = match side {
            DropSide::Before => anchor_index,
            DropSide::After => (anchor_index + 1).min(self.engine_folders.len()),
        };
        self.engine_folders.insert(insert_at, folder);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::container::{
        ContainerEngineKind, ContainerEngineProfile, ContainerEngineRef,
    };

    fn profile(name: &str) -> ContainerEngineProfile {
        ContainerEngineProfile::new(
            name,
            ContainerEngineRef {
                kind: ContainerEngineKind::Docker,
                endpoint: crate::model::container::ContainerEndpoint::Auto,
            },
        )
    }

    #[test]
    fn engine_folder_tree_and_counts_survive_delete() {
        let mut catalog = SessionCatalog::default();
        let prod = catalog.add_engine_folder("生产", None).unwrap();
        let db = catalog.add_engine_folder("数据库", Some(prod)).unwrap();
        let local = profile("本地 Docker");
        let id = local.id;
        catalog.engines.push(local);
        catalog.move_engine(id, Some(db)).unwrap();
        assert_eq!(
            catalog.engine_folder_path(db).as_deref(),
            Some("生产 / 数据库")
        );
        assert_eq!(catalog.engine_folder_of(id).map(|f| f.id), Some(db));

        let removed = catalog.delete_engine_folder(prod).unwrap();
        assert_eq!(removed, "生产");
        assert!(catalog.engine_folder(prod).is_none());
        assert_eq!(catalog.engine_folder(db).and_then(|f| f.parent), None);
        assert_eq!(catalog.engine_folder_of(id).map(|f| f.id), Some(db));
    }

    #[test]
    fn engine_folder_cycle_is_rejected() {
        let mut catalog = SessionCatalog::default();
        let prod = catalog.add_engine_folder("生产", None).unwrap();
        let db = catalog.add_engine_folder("数据库", Some(prod)).unwrap();
        assert!(catalog.move_engine_folder(prod, Some(db)).is_err());
    }

    #[test]
    fn reorder_engine_moves_into_anchor_folder() {
        let mut catalog = SessionCatalog::default();
        let folder = catalog.add_engine_folder("远端", None).unwrap();
        let first = profile("A");
        let second = profile("B");
        let (a, b) = (first.id, second.id);
        catalog.engines.push(first);
        catalog.engines.push(second);
        catalog.move_engine(b, Some(folder)).unwrap();
        catalog.reorder_engine(a, b, DropSide::After).unwrap();
        assert_eq!(
            catalog
                .engines
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>(),
            vec!["B", "A"]
        );
        assert_eq!(catalog.engines[1].folder_id, Some(folder));
    }
}
