//! Git 侧栏分组管理与拖放排序动作。
//!
//! 设计原理（Why）：侧栏的分组增删改查与拖放重排逻辑属于界面组织层交互，
//! 与仓库内部的 Git 动作（提交、拉取、推送等）生命周期正交；单独拆出避免
//! `actions.rs` 文件行数超过 500 行上限。

use super::super::types::{GitDropTarget, GitHub, GitSidebarDrag};
use transom_core::{GitFolder, GitFolderId, GitProjectId};

impl GitHub {
    /// 选中 Git 侧栏项目；返回是否发生变更。
    pub(crate) fn select_project(&mut self, id: GitProjectId) -> bool {
        if self.selected_project == Some(id) {
            return false;
        }
        self.selected_project = Some(id);
        true
    }

    /// 新建分组（名字的非空校验在对话框里完成）。
    pub(crate) fn add_folder(&mut self, name: String) {
        self.folders.push(GitFolder::new(name));
    }

    /// 重命名分组；返回是否找到并更新了目标分组。
    pub(crate) fn rename_folder(&mut self, id: GitFolderId, name: String) -> bool {
        let Some(folder) = self.folders.iter_mut().find(|item| item.id == id) else {
            return false;
        };
        folder.name = name;
        true
    }

    /// 删除分组：组内项目退回根层，磁盘上的仓库一个都不动。
    pub(crate) fn delete_folder(&mut self, id: GitFolderId) {
        crate::git::helpers::delete_git_folder(&mut self.projects, &mut self.folders, id);
    }

    /// 折叠或展开分组；返回是否找到并更新了目标分组。
    pub(crate) fn toggle_folder_collapsed(&mut self, id: GitFolderId) -> bool {
        let Some(folder) = self.folders.iter_mut().find(|item| item.id == id) else {
            return false;
        };
        folder.collapsed = !folder.collapsed;
        true
    }

    /// 侧栏拖放落点：把落点翻译成一次列表改动，返回是否发生了实际改动（需落盘）。
    pub(crate) fn apply_drop(&mut self, drag: GitSidebarDrag, target: GitDropTarget) -> bool {
        let mut changed = false;
        let mut landed_folder: Option<GitFolderId> = None;
        match (drag, target) {
            (GitSidebarDrag::Project(id), GitDropTarget::Folder(folder))
            | (
                GitSidebarDrag::Project(id),
                GitDropTarget::Relative {
                    anchor: GitSidebarDrag::Folder(folder),
                    ..
                },
            ) => {
                changed =
                    crate::git::helpers::move_git_project(&mut self.projects, id, Some(folder));
                if changed {
                    landed_folder = Some(folder);
                }
            }
            (
                GitSidebarDrag::Project(id),
                GitDropTarget::Relative {
                    anchor: GitSidebarDrag::Project(anchor),
                    side,
                },
            ) => {
                changed =
                    crate::git::helpers::reorder_git_projects(&mut self.projects, id, anchor, side);
            }
            (
                GitSidebarDrag::Folder(id),
                GitDropTarget::Relative {
                    anchor: GitSidebarDrag::Folder(anchor),
                    side,
                },
            ) => {
                changed =
                    crate::git::helpers::reorder_git_folders(&mut self.folders, id, anchor, side);
            }
            _ => {}
        }
        if let Some(folder) = landed_folder
            && let Some(folder) = self.folders.iter_mut().find(|item| item.id == folder)
        {
            folder.collapsed = false;
        }
        changed
    }

    /// 把项目移出分组，退回根层；返回是否发生了实际改动。
    pub(crate) fn move_project_to_root(&mut self, id: GitProjectId) -> bool {
        crate::git::helpers::move_git_project(&mut self.projects, id, None)
    }
}
