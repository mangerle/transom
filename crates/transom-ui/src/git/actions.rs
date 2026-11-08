//! Git 面板的动作派发与事件落地的公共层：项目列表、标签页生命周期与状态登记。
//!
//! 设计原理（Why）：
//! - 界面只发命令、只收事件，绝不直接跑子进程；所有方法都是「改状态 + 发命令」
//!   两步，失败路径统一由 `ReadFailed` / `ActionDone` 事件带回，避免在每个
//!   点击回调里各写一遍错误处理；
//! - 事件按 `project_id` 落到对应标签页：同一仓库可能在两个窗口/两个标签里
//!   同时打开，逐个标签匹配而不是记住「最后一次请求属于哪个标签」，才不会
//!   在快速切换时把 A 仓库的日志写进 B 仓库的视图；
//! - 写动作成功后立刻自动刷新：git 的索引是外部可变的（用户可能在命令行里
//!   同时操作），让界面停留在动作前的快照等于显示假数据；
//! - 具体动作按对象分组拆成同级子模块（`log_actions` / `ref_actions` /
//!   `shelf_actions` / `event_dispatch`），本文件只留项目与标签这一公共层，
//!   并对外再导出各模块的类型：调用方的引用路径不因拆分而改变。

use super::GitWorkspace;
use super::commit_modal::{ChangeGroup, worktree_diff_request};
use super::form::GitProjectDraft;
use super::types::{GitBadge, GitWorkspaceState};
use crate::app::{AppShell, TerminalTab};
use crate::palette::localized_error;
use gpui_kit::*;
use transom_core::{
    Connection, GitAction, GitCommand, GitProject, GitProjectId, GitRequestId, LocalConnection,
    Session, TabId,
};

mod diff_actions;
mod event_dispatch;
mod init_event;
mod log_actions;
mod ref_actions;
mod shelf_actions;
mod sidebar_actions;

// 对外形状保持不变：这些类型仍按 `crate::git::actions::*` 引用；
// 方法本身是 `impl AppShell` 的固有方法，拆到子模块后调用方写法不变。
pub(crate) use super::types::{GitDiffRequest, first_worktree_diff};
pub(crate) use ref_actions::{GitHistoryAction, GitRefAction, GitRemoteAction};

impl AppShell {
    /// 项目与分组一次落盘。
    ///
    /// 设计原理（Why）：两者必须同一次写完——分开保存会出现「分组已删、
    /// 项目的 `folder_id` 还指着已消失分组」这种撕裂状态（同 `SessionStore` 的理由）。
    pub(crate) fn persist_git_projects(&mut self, cx: &mut Context<Self>) {
        let registry = transom_core::GitRegistry {
            projects: self.state.git.projects.clone(),
            folders: self.state.git.folders.clone(),
        };
        if let Err(error) = self.state.services.git_projects.save(&registry) {
            self.set_error(localized_error(&error));
        }
        cx.notify();
    }

    /// 活动栏点击：切换左侧槽位的 Git 项目列表。
    pub(crate) fn toggle_git_sidebar(&mut self, cx: &mut Context<Self>) {
        let opening = !(self.state.git.sidebar_open && self.state.sidebar.visible);
        self.state.git.sidebar_open = opening;
        self.state.sidebar.visible = opening;
        if opening {
            self.state.vertical_tabs_open = false;
            // 展开即刷一遍所有项目：列表里的分支与脏数必须是当下的。
            self.refresh_all_git_projects(cx);
        }
        cx.notify();
    }

    /// 刷新全部已登记项目的状态徽标。
    pub(crate) fn refresh_all_git_projects(&mut self, cx: &mut Context<Self>) {
        for index in 0..self.state.git.projects.len() {
            let project = self.state.git.projects[index].clone();
            self.git_refresh(&project, cx);
        }
    }

    /// 采集单个项目的状态快照（侧栏徽标与标签页共用同一条命令）。
    pub(crate) fn git_refresh(&mut self, project: &GitProject, cx: &mut Context<Self>) {
        let badge = self.state.git.badges.entry(project.id).or_default();
        badge.loading = true;
        badge.error = None;
        self.send_command(GitCommand::Refresh {
            project: project.clone(),
        });
        self.send_command(GitCommand::LoadRefs {
            project: project.clone(),
        });
        self.git_reload_worktrees(project, cx);
        cx.notify();
    }

    /// 打开某个改动行的差异（文件行按钮、行点击与首屏自动选中共用）。
    pub(crate) fn git_open_worktree_diff(
        &mut self,
        project: &GitProject,
        group: ChangeGroup,
        path: String,
        cx: &mut Context<Self>,
    ) {
        let Some(tab_id) = self.git_tab_for_project(project.id, cx) else {
            return;
        };
        self.git_open_diff(tab_id, worktree_diff_request(group, path), cx);
    }

    /// 勾选/取消「修正」：勾上时回填目标提交信息并读取其文件清单。
    pub(crate) fn git_toggle_commit_amend(
        &mut self,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.state.git_workspace(tab_id) else {
            return;
        };
        let pending = workspace.update(cx, |workspace, _| {
            let state = &mut workspace.state;
            let next = !state.commit_amend;
            state.commit_amend = next;
            if !next {
                state.amend_files_loading = false;
                return None;
            }
            let target_oid = state.amend_target.clone();
            let entry = target_oid
                .as_ref()
                .and_then(|oid| state.log.entries.iter().find(|e| &e.oid == oid))
                .or_else(|| state.log.entries.first())
                .cloned()?;
            let mut message = entry.subject.clone();
            let body = entry.body.trim();
            if !body.is_empty() {
                message.push_str("\n\n");
                message.push_str(body);
            }
            let request_id = GitRequestId::new();
            state.amend_request = Some(request_id);
            state.amend_files_loading = true;
            state.amend_files.clear();
            state.pending.push(request_id);
            Some((
                state.project.clone(),
                request_id,
                message,
                entry.oid.clone(),
                state.commit_message.clone(),
            ))
        });
        if let Some((project, request_id, message, oid, commit_message)) = pending {
            commit_message.update(cx, |input, cx| input.set_value(message, window, cx));
            self.send_command(GitCommand::LoadCommitFiles {
                project,
                request_id,
                oid,
            });
        }
        cx.notify();
    }

    /// 选择修正目标提交：更新 amend_target，若当前已开启修正则回填消息并加载文件。
    pub(crate) fn git_select_amend_target(
        &mut self,
        tab_id: TabId,
        oid: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.state.git_workspace(tab_id) else {
            return;
        };
        let pending = workspace.update(cx, |workspace, _| {
            let state = &mut workspace.state;
            state.amend_target = Some(oid.clone());
            if !state.commit_amend {
                return None;
            }
            let entry = state
                .log
                .entries
                .iter()
                .find(|e| e.oid == oid)
                .or_else(|| state.log.entries.first())
                .cloned()?;
            let mut message = entry.subject.clone();
            let body = entry.body.trim();
            if !body.is_empty() {
                message.push_str("\n\n");
                message.push_str(body);
            }
            let request_id = GitRequestId::new();
            state.amend_request = Some(request_id);
            state.amend_files_loading = true;
            state.amend_files.clear();
            state.pending.push(request_id);
            Some((
                state.project.clone(),
                request_id,
                message,
                entry.oid.clone(),
                state.commit_message.clone(),
            ))
        });
        if let Some((project, request_id, message, oid, commit_message)) = pending {
            commit_message.update(cx, |input, cx| input.set_value(message, window, cx));
            self.send_command(GitCommand::LoadCommitFiles {
                project,
                request_id,
                oid,
            });
        }
        cx.notify();
    }

    /// 打开当前改动列表里第一个文件的差异（工具条「查看差异」）。
    pub(crate) fn git_open_first_worktree_diff(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(workspace) = self.state.git_workspace(tab_id) else {
            return;
        };
        let Some((group, path)) = ({
            let state = &workspace.read(cx).state;
            state.status.as_ref().and_then(first_worktree_diff)
        }) else {
            return;
        };
        self.git_open_diff(tab_id, worktree_diff_request(group, path), cx);
    }

    /// 回滚勾选中的改动：已跟踪走 Discard，未跟踪走 DeleteUntracked。
    pub(crate) fn git_rollback_checked(&mut self, project: &GitProject, cx: &mut Context<Self>) {
        let Some(workspace) = self.state.git_workspace_of_project(project.id, cx) else {
            return;
        };
        let (tracked, untracked, empty) = {
            let state = &workspace.read(cx).state;
            if state.checked.is_empty() {
                (Vec::new(), Vec::new(), true)
            } else {
                let mut tracked = Vec::new();
                let mut untracked = Vec::new();
                if let Some(status) = state.status.as_ref() {
                    for entry in &status.entries {
                        if !state.checked.contains(&entry.path) {
                            continue;
                        }
                        if matches!(entry.record, transom_core::StatusRecord::Untracked) {
                            untracked.push(entry.path.clone());
                        } else {
                            tracked.push(entry.path.clone());
                        }
                    }
                }
                (tracked, untracked, false)
            }
        };
        if empty {
            self.set_error(rust_i18n::t!("git.commit.nothing_selected").to_string());
            return;
        }
        if !tracked.is_empty() {
            self.git_run_paths(project, GitAction::Discard, tracked, cx);
        }
        if !untracked.is_empty() {
            self.git_run_paths(project, GitAction::DeleteUntracked, untracked, cx);
        }
        cx.notify();
    }

    /// 回滚单个文件路径的更改。
    pub(crate) fn git_rollback_single_path(
        &mut self,
        project: &GitProject,
        path: &str,
        cx: &mut Context<Self>,
    ) {
        self.git_run_paths(project, GitAction::Discard, vec![path.to_string()], cx);
    }

    /// 提交当前标签页的已暂存内容；`push_after` 决定是否顺带推送。
    pub(crate) fn submit_git_commit(
        &mut self,
        tab_id: TabId,
        push_after: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.state.git_workspace(tab_id) else {
            return;
        };
        let (message, project, amend) = {
            let state = &workspace.read(cx).state;
            (
                state.commit_message.read(cx).value().to_string(),
                state.project.clone(),
                state.commit_amend,
            )
        };
        if message.trim().is_empty() && !amend {
            self.set_error(rust_i18n::t!("git.commit.empty_message").to_string());
            return;
        }
        workspace.update(cx, |workspace, _| {
            let state = &mut workspace.state;
            state.commit_submitting = true;
            state.checked.clear();
        });
        self.git_commit(&project, message, amend, push_after, cx);
        cx.notify();
    }

    /// 对已登记但尚未初始化的目录补一次 `git init`。
    ///
    /// 成功后由 `Inited` 事件统一刷新状态与日志；失败时项目仍留在列表里，
    /// 用户可以修好目录权限后再次点击，不在失败路径上删登记。
    pub(crate) fn git_init_project(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(workspace) = self.state.git_workspace(tab_id) else {
            return;
        };
        let (init_running, project) = {
            let state = &workspace.read(cx).state;
            (state.init_running, state.project.clone())
        };
        if init_running {
            return;
        }
        let request_id = GitRequestId::new();
        workspace.update(cx, |workspace, _| {
            let state = &mut workspace.state;
            state.init_running = true;
            state.status_error = None;
        });
        self.state.git.pending_project_init = Some((request_id, project.id));
        self.send_command(GitCommand::Init {
            request_id,
            path: project.path,
        });
        cx.notify();
    }

    /// 打开（或聚焦）一个项目的 Git 标签页。
    pub(crate) fn open_git_tab(
        &mut self,
        project: GitProject,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(tab_id) = self.git_tab_for_project(project.id, cx) {
            self.activate_tab(tab_id, window, cx);
            return;
        }
        let tab_id = TabId::new();
        let session = git_placeholder_session(&project);
        let session_id = session.id;
        let title = project.display_name();
        let state = GitWorkspaceState::new(project.clone(), window, cx);
        let command_tx = self.state.services.command_tx.clone();
        let workspace = cx.new(|_| GitWorkspace::new(state, tab_id, command_tx));
        self.state
            .workspace
            .tabs
            .push(TerminalTab::git_tab(tab_id, session, title, workspace));
        self.state.workspace.create_group(tab_id, session_id);
        self.activate_tab(tab_id, window, cx);
        self.git_refresh(&project, cx);
        self.git_load_log(tab_id, 0, cx);
        cx.notify();
    }

    /// 关闭标签页时取消该项目所有在途请求。
    pub(crate) fn cancel_git_tab(&mut self, tab_id: TabId, cx: &mut Context<Self>) {
        let Some(workspace) = self.state.git_workspace(tab_id) else {
            return;
        };
        let requests = workspace.read(cx).state.pending.clone();
        for request_id in requests {
            self.send_command(GitCommand::Cancel { request_id });
        }
        cx.notify();
    }

    /// 按项目 id 找已打开的标签页。
    pub(crate) fn git_tab_for_project(&self, project_id: GitProjectId, cx: &App) -> Option<TabId> {
        self.state
            .git_workspace_of_project(project_id, cx)
            .map(|workspace| workspace.read(cx).tab_id)
    }

    /// 按草稿添加项目到列表（探测或初始化通过后调用）。
    pub(crate) fn add_git_project(&mut self, draft: GitProjectDraft, cx: &mut Context<Self>) {
        if self
            .state
            .git
            .projects
            .iter()
            .any(|item| item.path == draft.path)
        {
            self.set_error(rust_i18n::t!("git.project.duplicate").to_string());
            return;
        }
        let project = draft.into_project();
        self.state
            .git
            .badges
            .insert(project.id, GitBadge::default());
        self.state.git.projects.push(project.clone());
        self.persist_git_projects(cx);
        self.git_refresh(&project, cx);
    }

    /// 从列表移除项目（只删登记，不动磁盘）。
    pub(crate) fn remove_git_project(&mut self, id: GitProjectId, cx: &mut Context<Self>) {
        self.state.git.projects.retain(|project| project.id != id);
        self.state.git.badges.remove(&id);
        if let Some(tab_id) = self.git_tab_for_project(id, cx) {
            self.cancel_git_tab(tab_id, cx);
        }
        self.persist_git_projects(cx);
        cx.notify();
    }

    /// 打开偏好设置弹窗并直接定位到 Git 版本控制设置页。
    pub(crate) fn open_git_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_settings_modal(Some(crate::settings::GIT_PAGE_INDEX), window, cx);
    }
}

/// Git 标签页需要的占位会话。
///
/// 设计原理（Why）：工作区的标签、分组与关闭逻辑都以 `Session` 为轴，Git 面板
/// 不是终端但必须复用这套容器；用 `Connection::Local` 且工作目录指向仓库根，
/// 语义上就是「这个仓库在本机」，不引入第三种连接类型去牵动转发/SFTP 的分支。
fn git_placeholder_session(project: &GitProject) -> Session {
    Session::new(
        project.display_name(),
        Connection::Local(LocalConnection {
            shell: None,
            working_dir: Some(project.path.clone()),
            command: None,
        }),
    )
}
