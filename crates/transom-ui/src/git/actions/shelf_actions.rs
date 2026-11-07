//! 工作树与搁置（stash）的动作。
//!
//! 设计原理（Why）：两者都是「项目之外的对象」——工作树是另一个检出目录，
//! 搁置是栈里的快照，交互上都需要展开/刷新这类列表动作，放一起成模块。

use crate::app::AppShell;
use crate::git::GitCommitTab;
use gpui_kit::*;
use transom_core::{GitAction, GitCommand, GitProject, GitRequestId, GitSide, TabId};

use super::{GitDiffRequest, GitRefAction};

impl AppShell {
    /// 在提交页左栏切换「提交 / 搁置 / 贮藏」；首次进入分别拉取对应列表。
    pub(crate) fn git_set_commit_tab(
        &mut self,
        tab_id: TabId,
        tab: GitCommitTab,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.git_state_of_tab_mut(tab_id) else {
            return;
        };
        if state.commit_tab == tab {
            return;
        }
        state.commit_tab = tab;
        match tab {
            GitCommitTab::Shelf => {
                let project_id = state.project.id.to_string();
                if let Ok(shelves) = transom_store::load_project_shelves(&project_id) {
                    state.shelves = shelves;
                }
            }
            GitCommitTab::Stash => {
                let project = state.project.clone();
                let request_id = GitRequestId::new();
                state.pending.push(request_id);
                self.send_command(GitCommand::ListStashes {
                    project,
                    request_id,
                });
            }
            GitCommitTab::Commit => {}
        }
        cx.notify();
    }

    /// 拉取工作树列表。
    pub(crate) fn git_reload_worktrees(&mut self, project: &GitProject, cx: &mut Context<Self>) {
        let request_id = GitRequestId::new();
        if let Some(state) = self.git_state_mut(project.id) {
            state.pending.push(request_id);
        }
        self.send_command(GitCommand::ListWorktrees {
            project: project.clone(),
            request_id,
        });
        cx.notify();
    }

    /// 按对话框填好的参数新建工作树。
    pub(crate) fn git_create_worktree(
        &mut self,
        project: &GitProject,
        path: String,
        branch: String,
        start: Option<String>,
        checkout: bool,
        cx: &mut Context<Self>,
    ) {
        // 目录可能是相对仓库父目录的简写：补成绝对路径再交给 git，
        // 否则子进程的工作目录决定最终落点，用户看到的和实际创建的位置会偏。
        let target = {
            let candidate = std::path::PathBuf::from(&path);
            if candidate.is_absolute() {
                candidate
            } else {
                project
                    .path
                    .parent()
                    .map(|dir| dir.join(&candidate))
                    .unwrap_or(candidate)
            }
        };
        let text = target.display().to_string();
        if let Some(state) = self.git_state_mut(project.id) {
            state.pending_worktree = Some((text.clone(), checkout));
        }
        self.git_ref_action(
            project,
            GitRefAction::NewWorktree {
                path: text,
                branch,
                start,
            },
            cx,
        );
    }

    /// 锁定或解锁指定项目下的一个工作树。
    pub(crate) fn git_set_project_worktree_locked(
        &mut self,
        project: &GitProject,
        path: String,
        locked: bool,
        cx: &mut Context<Self>,
    ) {
        self.send_command(if locked {
            GitCommand::LockWorktree {
                project: project.clone(),
                path,
                reason: None,
            }
        } else {
            GitCommand::UnlockWorktree {
                project: project.clone(),
                path,
            }
        });
        cx.notify();
    }

    /// 删除工作树。
    pub(crate) fn git_remove_worktree(
        &mut self,
        project: &GitProject,
        path: String,
        force: bool,
        cx: &mut Context<Self>,
    ) {
        self.send_command(GitCommand::RemoveWorktree {
            project: project.clone(),
            path,
            force,
        });
        cx.notify();
    }

    /// 把一个工作树目录加进项目列表（IDEA 里点工作树就是切过去看）。
    pub(crate) fn git_open_worktree_path(&mut self, path: &str, cx: &mut Context<Self>) {
        let target = std::path::PathBuf::from(path);
        if let Some(project) = self
            .state
            .git
            .projects
            .iter()
            .find(|item| item.path == target)
            .cloned()
        {
            let tab_id = self.git_tab_for_project(project.id);
            if let Some(tab_id) = tab_id {
                let _ = tab_id;
            }
            self.git_refresh(&project, cx);
            return;
        }
        self.add_git_project(super::super::GitProjectDraft::new(target), cx);
    }

    /// 新建搁置项。
    pub(crate) fn git_stash_save(
        &mut self,
        project: &GitProject,
        message: String,
        include_untracked: bool,
        cx: &mut Context<Self>,
    ) {
        self.send_command(GitCommand::StashSave {
            project: project.clone(),
            message,
            include_untracked,
        });
        cx.notify();
    }

    /// 搁置项动作：应用 / 弹出 / 丢弃。
    pub(crate) fn git_stash_action(
        &mut self,
        project: &GitProject,
        action: GitAction,
        selector: String,
        cx: &mut Context<Self>,
    ) {
        let command = match action {
            GitAction::StashApply => GitCommand::StashApply {
                project: project.clone(),
                selector,
            },
            GitAction::StashPop => GitCommand::StashPop {
                project: project.clone(),
                selector,
            },
            GitAction::StashDrop => GitCommand::StashDrop {
                project: project.clone(),
                selector,
            },
            other => {
                log::error!("非预期的搁置动作: {other:?}");
                return;
            }
        };
        self.send_command(command);
        cx.notify();
    }

    /// 展开/收起一条搁置项的文件清单。
    ///
    /// 为什么每次展开都重新读而不缓存：搁置栈的编号会随删除整体前移，
    /// `stash@{0}` 前后两秒可能指向不同条目，缓存省一次子进程却会读到过期清单；
    /// 展开是一次点击的成本，不做这个优化。
    pub(crate) fn git_toggle_stash_files(
        &mut self,
        project: &GitProject,
        selector: &str,
        cx: &mut Context<Self>,
    ) {
        let request = self.git_state_mut(project.id).and_then(|state| {
            let opening = state.expanded_stash.as_deref() != Some(selector);
            state.expanded_stash = opening.then(|| selector.to_string());
            state.stash_files.clear();
            state.stash_files_loading = false;
            if !opening {
                return None;
            }
            let request_id = GitRequestId::new();
            state.pending.push(request_id);
            state.stash_files_loading = true;
            Some(request_id)
        });
        let Some(request_id) = request else {
            cx.notify();
            return;
        };
        self.send_command(GitCommand::ListStashFiles {
            project: project.clone(),
            request_id,
            selector: selector.to_string(),
        });
        cx.notify();
    }

    /// 打开某个搁置项里一个文件的差异：搁置前的基线对搁置后的内容。
    ///
    /// 未跟踪条目住在第三个父提交（`stash@{n}^3`）里，用普通侧去取会两边都空，
    /// 所以按 `untracked` 标记选侧。
    pub(crate) fn git_open_stash_diff(
        &mut self,
        tab_id: TabId,
        selector: &str,
        file: &transom_core::StashFile,
        cx: &mut Context<Self>,
    ) {
        let stashed = if file.untracked {
            format!("{selector}^3")
        } else {
            selector.to_string()
        };
        self.git_open_diff(
            tab_id,
            GitDiffRequest {
                path: file.path.clone(),
                old: GitSide::Commit(format!("{selector}^")),
                new: GitSide::Commit(stashed),
                left_label: rust_i18n::t!("git.diff.baseline").to_string(),
                right_label: rust_i18n::t!("git.diff.stashed").to_string(),
            },
            cx,
        );
    }
}
