//! Git 实体弹窗与 AppShell 之间的生命周期与事件桥接。
//!
//! 设计原理（Why）：
//! - 弹窗视图实体（如 `GitRebaseModalView`、`GitConflictModalView`、`GitPushModalView`）
//!   仅维护弹窗内部状态与 UI 渲染，通过 `EventEmitter` 上报需要外壳协调的跨模块命令；
//! - 由本桥接模块集中创建实体、登记订阅、转发后台 `GitCommand` 与回流结果。

use crate::app::AppShell;
use crate::git::conflict_modal::{GitConflictModalEvent, GitConflictModalView};
use crate::git::push_modal::{GitPushModalEvent, GitPushModalView};
use crate::git::rebase_modal::{GitRebaseModalEvent, GitRebaseModalView};
use gpui_kit::*;
use transom_core::ConflictSides;
use transom_core::{CommitEntry, CommitFile, HeadState};
use transom_core::{GitAction, GitCommand, GitProject, GitProjectId, GitRebaseStep, GitRequestId};
mod clone;
mod lightweight;
mod project;
pub(crate) use clone::*;
pub(crate) use lightweight::*;
pub(crate) use project::*;

/// 打开「从这里交互式变基」弹窗实体并向后台请求待办清单。
/// Git 模态桥接的窄操作上下文。
pub(crate) struct GitModalActions<'a> {
    state: &'a mut crate::app::AppState,
}

impl<'a> GitModalActions<'a> {
    /// 从应用状态拆借 Git 模态协调能力。
    pub(crate) fn new(state: &'a mut crate::app::AppState) -> Self {
        Self { state }
    }

    fn send_command(&mut self, command: impl Into<transom_core::AppCommand>) -> bool {
        self.state.send_command(command)
    }
}

impl<'a> GitModalActions<'a> {
    pub(crate) fn open_git_rebase_modal(
        &mut self,
        project: &GitProject,
        base: &str,
        cx: &mut Context<AppShell>,
    ) {
        let subject = self
            .state
            .git_workspace_of_project(project.id, cx)
            .and_then(|workspace| {
                let state = &workspace.read(cx).state;
                state
                    .log
                    .entries
                    .iter()
                    .find(|entry| entry.oid == base)
                    .map(|entry| entry.subject.clone())
            })
            .unwrap_or_default();
        let request_id = GitRequestId::new();
        if let Some(workspace) = self.state.git_workspace_of_project(project.id, cx) {
            workspace.update(cx, |workspace, _| {
                workspace.state.pending.push(request_id);
            });
        }
        let modal = cx.new(|_| GitRebaseModalView::new(project.clone(), base.to_string(), subject));
        let subscription = cx.subscribe(&modal, |shell, _, event: &GitRebaseModalEvent, cx| {
            GitModalActions::new(&mut shell.state).on_rebase_modal_event(event, cx);
        });
        self.state.ui.modals.git.rebase = Some(modal);
        self.state.runtime.tasks.subscriptions.git_rebase_modal = Some(subscription);
        self.send_command(GitCommand::RebaseTodo {
            project: project.clone(),
            request_id,
            base: base.to_string(),
        });
        cx.notify();
    }

    fn on_rebase_modal_event(&mut self, event: &GitRebaseModalEvent, cx: &mut Context<AppShell>) {
        match event {
            GitRebaseModalEvent::Run {
                project,
                base,
                steps,
            } => {
                self.send_command(GitCommand::RebaseRun {
                    project: project.clone(),
                    base: base.clone(),
                    steps: steps.clone(),
                });
                cx.notify();
            }
            GitRebaseModalEvent::Close => {
                self.close_git_rebase_modal(cx);
            }
        }
    }

    /// 把后台取回的变基待办清单回填给弹窗实体。
    pub(crate) fn apply_git_rebase_todo(
        &mut self,
        base: &str,
        steps: Vec<GitRebaseStep>,
        cx: &mut Context<AppShell>,
    ) {
        if let Some(modal) = self.state.ui.modals.git.rebase.as_ref() {
            modal.update(cx, |modal, cx| modal.apply_todo(base, steps, cx));
        }
    }

    /// 变基执行失败时通知弹窗实体保留窗口并展示错误。
    pub(crate) fn fail_git_rebase_modal(
        &mut self,
        project_id: GitProjectId,
        action: GitAction,
        message: String,
        cx: &mut Context<AppShell>,
    ) {
        if let Some(modal) = self.state.ui.modals.git.rebase.as_ref() {
            modal.update(cx, |modal, cx| {
                modal.set_failed(project_id, action, message, cx)
            });
        }
    }

    /// 触发当前交互式变基弹窗提交。
    pub(crate) fn run_git_rebase_modal(&mut self, cx: &mut Context<AppShell>) {
        if let Some(modal) = self.state.ui.modals.git.rebase.as_ref() {
            modal.update(cx, |modal, cx| modal.run(cx));
        }
    }

    /// 关闭交互式变基弹窗（取消不改历史）。
    pub(crate) fn close_git_rebase_modal(&mut self, cx: &mut Context<AppShell>) {
        self.state.ui.modals.git.rebase = None;
        self.state.runtime.tasks.subscriptions.git_rebase_modal = None;
        cx.notify();
    }

    /// 打开冲突解决模态框实体并加载首个冲突文件的三方内容。
    pub(crate) fn open_git_conflict_modal(
        &mut self,
        project: &GitProject,
        cx: &mut Context<AppShell>,
    ) {
        let conflict_files: Vec<String> = self
            .state
            .git_workspace_of_project(project.id, cx)
            .and_then(|workspace| {
                workspace.read(cx).state.status.as_ref().map(|status| {
                    status
                        .conflicts()
                        .into_iter()
                        .map(|e| e.path.clone())
                        .collect()
                })
            })
            .unwrap_or_default();

        if conflict_files.is_empty() {
            return;
        }

        let first_file = conflict_files[0].clone();
        let modal = cx.new(|_| GitConflictModalView::new(project.clone(), conflict_files));
        let subscription = cx.subscribe(&modal, |shell, _, event: &GitConflictModalEvent, cx| {
            GitModalActions::new(&mut shell.state).on_conflict_modal_event(event, cx);
        });
        self.state.ui.modals.git.conflict = Some(modal);
        self.state.runtime.tasks.subscriptions.git_conflict_modal = Some(subscription);

        self.send_command(GitCommand::LoadConflictSides {
            project: project.clone(),
            request_id: GitRequestId::new(),
            path: first_file,
        });
        cx.notify();
    }

    fn on_conflict_modal_event(
        &mut self,
        event: &GitConflictModalEvent,
        cx: &mut Context<AppShell>,
    ) {
        match event {
            GitConflictModalEvent::LoadSides { project, path } => {
                self.send_command(GitCommand::LoadConflictSides {
                    project: project.clone(),
                    request_id: GitRequestId::new(),
                    path: path.clone(),
                });
                cx.notify();
            }
            GitConflictModalEvent::ResolveOurs { project, path } => {
                self.send_command(GitCommand::ResolveConflictOurs {
                    project: project.clone(),
                    paths: vec![path.clone()],
                });
                cx.notify();
            }
            GitConflictModalEvent::ResolveTheirs { project, path } => {
                self.send_command(GitCommand::ResolveConflictTheirs {
                    project: project.clone(),
                    paths: vec![path.clone()],
                });
                cx.notify();
            }
            GitConflictModalEvent::MarkResolved { project, path } => {
                self.send_command(GitCommand::ResolveConflictMark {
                    project: project.clone(),
                    paths: vec![path.clone()],
                });
                cx.notify();
            }
            GitConflictModalEvent::Close => self.close_git_conflict_modal(cx),
        }
    }

    /// 将后台返回的三方冲突内容落盘到弹窗实体。
    pub(crate) fn apply_git_conflict_sides(
        &mut self,
        project_id: GitProjectId,
        path: &str,
        sides: ConflictSides,
        cx: &mut Context<AppShell>,
    ) {
        if let Some(modal) = self.state.ui.modals.git.conflict.as_ref() {
            modal.update(cx, |modal, cx| {
                modal.apply_sides(project_id, path, sides, cx)
            });
        }
    }

    /// 处理冲突解决写动作完成事件。
    pub(crate) fn handle_git_conflict_resolve_done(
        &mut self,
        error_message: Option<String>,
        cx: &mut Context<AppShell>,
    ) {
        if let Some(modal) = self.state.ui.modals.git.conflict.as_ref() {
            modal.update(cx, |modal, cx| modal.handle_resolve_done(error_message, cx));
        }
    }

    /// 关闭冲突解决模态框。
    pub(crate) fn close_git_conflict_modal(&mut self, cx: &mut Context<AppShell>) {
        self.state.ui.modals.git.conflict = None;
        self.state.runtime.tasks.subscriptions.git_conflict_modal = None;
        cx.notify();
    }

    /// 唤起 Git 推送模态弹窗实体并异步加载待推送提交。
    pub(crate) fn open_git_push_modal(
        &mut self,
        project: &GitProject,
        target_branch: Option<String>,
        cx: &mut Context<AppShell>,
    ) {
        let (branch_name, upstream_name) = self
            .state
            .git_workspace_of_project(project.id, cx)
            .map(|workspace| {
                let state = &workspace.read(cx).state;
                let head_branch = state
                    .status
                    .as_ref()
                    .and_then(|s| match &s.head {
                        HeadState::OnBranch(name) => Some(name.clone()),
                        _ => None,
                    })
                    .unwrap_or_else(|| "HEAD".to_string());
                let branch = target_branch.unwrap_or(head_branch);
                let upstream = state.status.as_ref().and_then(|s| s.upstream.clone());
                (branch, upstream)
            })
            .unwrap_or_else(|| ("HEAD".to_string(), None));

        let (remote, remote_branch) = if let Some(ref up) = upstream_name {
            if let Some((r, b)) = up.split_once('/') {
                (r.to_string(), b.to_string())
            } else {
                ("origin".to_string(), up.clone())
            }
        } else {
            ("origin".to_string(), branch_name.clone())
        };

        let tab_id = self
            .state
            .git_workspace_of_project(project.id, cx)
            .map(|workspace| workspace.read(cx).tab_id);
        let modal = cx.new(|_| {
            GitPushModalView::new(
                project.clone(),
                tab_id,
                branch_name.clone(),
                remote.clone(),
                remote_branch.clone(),
            )
        });
        let subscription = cx.subscribe(&modal, |shell, _, event: &GitPushModalEvent, cx| {
            GitModalActions::new(&mut shell.state).on_push_modal_event(event, cx);
        });
        self.state.ui.modals.git.push = Some(modal);
        self.state.runtime.tasks.subscriptions.git_push_modal = Some(subscription);

        let request_id = GitRequestId::new();
        if let Some(workspace) = self.state.git_workspace_of_project(project.id, cx) {
            workspace.update(cx, |workspace, _| {
                workspace.state.pending.push(request_id);
            });
        }
        self.send_command(GitCommand::LoadOutgoingCommits {
            project: project.clone(),
            request_id,
            branch: branch_name,
            remote: Some(remote),
            remote_branch: Some(remote_branch),
        });
        cx.notify();
    }

    fn on_push_modal_event(&mut self, event: &GitPushModalEvent, cx: &mut Context<AppShell>) {
        match event {
            GitPushModalEvent::LoadCommitFiles { project, oid } => {
                let request_id = GitRequestId::new();
                if let Some(workspace) = self.state.git_workspace_of_project(project.id, cx) {
                    workspace.update(cx, |workspace, _| {
                        workspace.state.pending.push(request_id);
                    });
                }
                self.send_command(GitCommand::LoadCommitFiles {
                    project: project.clone(),
                    request_id,
                    oid: oid.clone(),
                });
                cx.notify();
            }
            GitPushModalEvent::Submit { project, options } => {
                self.send_command(GitCommand::Push {
                    project: project.clone(),
                    options: options.clone(),
                });
                cx.notify();
            }
            GitPushModalEvent::OpenFileDiff { tab_id, path } => {
                if let Some(workspace) = self.state.git_workspace(*tab_id) {
                    workspace.update(cx, |ws, cx| ws.diff_commit_file(path, cx));
                }
            }
            GitPushModalEvent::CopyOid(oid) => {
                cx.write_to_clipboard(ClipboardItem::new_string(oid.to_string()));
                self.state
                    .set_transient_status(rust_i18n::t!("git.menu.copied"), cx);
                cx.notify();
            }
            GitPushModalEvent::Close => self.close_git_push_modal(cx),
        }
    }

    /// 待推送提交列表加载完成落盘到推送弹窗实体。
    pub(crate) fn apply_git_outgoing_commits(
        &mut self,
        project_id: GitProjectId,
        branch: &str,
        remote_branch: &str,
        commits: Vec<CommitEntry>,
        cx: &mut Context<AppShell>,
    ) {
        if let Some(modal) = self.state.ui.modals.git.push.as_ref() {
            modal.update(cx, |modal, cx| {
                modal.apply_outgoing_commits(project_id, branch, remote_branch, commits, cx)
            });
        }
    }

    /// 提交的文件清单加载完成落盘到推送弹窗实体。
    pub(crate) fn apply_git_push_commit_files(
        &mut self,
        oid: &str,
        files: Vec<CommitFile>,
        cx: &mut Context<AppShell>,
    ) {
        if let Some(modal) = self.state.ui.modals.git.push.as_ref() {
            modal.update(cx, |modal, cx| modal.apply_commit_files(oid, files, cx));
        }
    }

    /// 处理推送结果。
    pub(crate) fn handle_git_push_result(
        &mut self,
        project_id: GitProjectId,
        error: Option<String>,
        cx: &mut Context<AppShell>,
    ) {
        if let Some(modal) = self.state.ui.modals.git.push.as_ref() {
            modal.update(cx, |modal, cx| {
                modal.handle_push_result(project_id, error, cx)
            });
        }
    }

    /// 触发推送弹窗提交。
    pub(crate) fn submit_git_push_modal(&mut self, cx: &mut Context<AppShell>) {
        if let Some(modal) = self.state.ui.modals.git.push.as_ref() {
            modal.update(cx, |modal, cx| modal.submit(cx));
        }
    }

    /// 关闭推送模态弹窗。
    pub(crate) fn close_git_push_modal(&mut self, cx: &mut Context<AppShell>) {
        self.state.ui.modals.git.push = None;
        self.state.runtime.tasks.subscriptions.git_push_modal = None;
        cx.notify();
    }
}
