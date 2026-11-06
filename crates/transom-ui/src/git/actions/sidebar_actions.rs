//! Git 侧栏分组管理与拖放排序动作。
//!
//! 设计原理（Why）：侧栏的分组增删改查与拖放重排逻辑属于界面组织层交互，
//! 与仓库内部的 Git 动作（提交、拉取、推送等）生命周期正交；单独拆出避免
//! `actions.rs` 文件行数超过 500 行上限。

use super::super::types::{GitDropTarget, GitSidebarDrag};
use crate::app::AppShell;
use gpui_kit::*;
use transom_core::{GitFolder, GitFolderId, GitProjectId};

impl AppShell {
    /// 新建分组（名字的非空校验在对话框里完成）。
    pub(crate) fn add_git_folder(&mut self, name: String, cx: &mut Context<Self>) {
        self.state.git.folders.push(GitFolder::new(name));
        self.persist_git_projects(cx);
    }

    /// 重命名分组。
    pub(crate) fn rename_git_folder(
        &mut self,
        id: GitFolderId,
        name: String,
        cx: &mut Context<Self>,
    ) {
        let Some(folder) = self.state.git.folders.iter_mut().find(|item| item.id == id) else {
            return;
        };
        folder.name = name;
        self.persist_git_projects(cx);
    }

    /// 删除分组：组内项目退回根层，磁盘上的仓库一个都不动。
    pub(crate) fn delete_git_folder(&mut self, id: GitFolderId, cx: &mut Context<Self>) {
        crate::git::helpers::delete_git_folder(
            &mut self.state.git.projects,
            &mut self.state.git.folders,
            id,
        );
        self.persist_git_projects(cx);
    }

    /// 折叠或展开分组，并记住状态（分组的作用就是收纳，每次重启都要重收一遍等于没用）。
    pub(crate) fn toggle_git_folder_collapsed(&mut self, id: GitFolderId, cx: &mut Context<Self>) {
        let Some(folder) = self.state.git.folders.iter_mut().find(|item| item.id == id) else {
            return;
        };
        folder.collapsed = !folder.collapsed;
        self.persist_git_projects(cx);
    }

    /// 侧栏拖放落点：把落点翻译成一次列表改动。
    ///
    /// 设计原理（Why）：顺序是用户自己的优先级，没有任何可派生的排序键，
    /// 所以数组顺序就是唯一真相并在落点后立刻写盘；一旦在读取处再加一次排序，
    /// 拖拽结果会在下一次刷新时被静默覆盖（会话侧栏同样的坑已在 `tree/rows.rs` 记档）。
    /// 跨类型落点按"能落进容器"解释——项目落在分组行的任何区域
    /// 都算进这个组（用户没有表达组内顺序，追加到组末尾即可）；分组只能与分组换序，
    /// 落在项目行上直接忽略，因为单层分组下"把组放到某个项目旁边"没有可执行的语义。
    /// 落入的分组若处于折叠态会自动展开，保证拖进去的东西立刻看得见。
    pub(crate) fn apply_git_drop(
        &mut self,
        drag: GitSidebarDrag,
        target: GitDropTarget,
        cx: &mut Context<Self>,
    ) {
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
                changed = crate::git::helpers::move_git_project(
                    &mut self.state.git.projects,
                    id,
                    Some(folder),
                );
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
                changed = crate::git::helpers::reorder_git_projects(
                    &mut self.state.git.projects,
                    id,
                    anchor,
                    side,
                );
            }
            (
                GitSidebarDrag::Folder(id),
                GitDropTarget::Relative {
                    anchor: GitSidebarDrag::Folder(anchor),
                    side,
                },
            ) => {
                changed = crate::git::helpers::reorder_git_folders(
                    &mut self.state.git.folders,
                    id,
                    anchor,
                    side,
                );
            }
            // 其余组合（分组落到项目行、分组落进另一个分组）在单层分组下没有
            // 可执行的落点语义：直接忽略，不猜用户想要哪种顺序。
            _ => {}
        }
        if let Some(folder) = landed_folder
            && let Some(folder) = self
                .state
                .git
                .folders
                .iter_mut()
                .find(|item| item.id == folder)
        {
            folder.collapsed = false;
        }
        // 只在真的改动过才落盘：拖回原位这种 no-op 不该触发一次写文件。
        if changed {
            self.persist_git_projects(cx);
        }
        cx.notify();
    }

    /// 把项目移出分组，退回根层（右键菜单里「移到根层」的落点）。
    pub(crate) fn git_move_project_to_root(&mut self, id: GitProjectId, cx: &mut Context<Self>) {
        if crate::git::helpers::move_git_project(&mut self.state.git.projects, id, None) {
            self.persist_git_projects(cx);
        }
        cx.notify();
    }
}
