//! 目录节点拖拽排序、快照合并与结构校验归一化。

use super::{CatalogError, CatalogMergeSummary, DropSide, SessionCatalog, SessionFolder};
use crate::ids::{FolderId, SessionId};
use std::collections::{HashMap, HashSet};

impl SessionCatalog {
    /// 将会话移到锚点会话所在层级，并插到其前面或后面。
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

    /// 合并另一份目录快照，用于备份导入与多机同步。
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

    /// 修复外部文件中的损坏树结构，保证后续 UI 与移动操作安全。
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
