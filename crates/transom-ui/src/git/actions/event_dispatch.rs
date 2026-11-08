//! Git 事件落地：把控制器回传的事件写进对应标签页与侧栏徽标。
//!
//! 设计原理（Why）：事件处理是「唯一写界面的入口」，单独成模块便于逐条核对
//! 迟到请求的丢弃口径（按 `request_id` 比对）与写动作后的自动刷新，
//! 不让这些规则散落在各个点击回调里。
//!
//! 状态写入一律走 `Entity<GitWorkspace>::update`：闭包内只改该标签页的状态并把
//! 外壳后续需要的数据带出来，闭包结束后再调用外壳方法，避免同一实体被二次租借。

use crate::app::AppShell;
use crate::palette::localized_error;
use gpui_kit::*;
use std::collections::HashSet;
use transom_core::{ErrorCode, GitAction, GitAppEvent, GitCommand, GitProjectId};

impl AppShell {
    /// Git 事件落地。
    pub(crate) fn dispatch_git_event(&mut self, event: GitAppEvent, cx: &mut Context<Self>) {
        match event {
            GitAppEvent::StatusLoaded { project_id, status } => {
                self.apply_git_status(project_id, *status, cx);
            }
            GitAppEvent::RefsLoaded {
                project_id,
                branches,
                tags,
                paths,
            } => {
                if let Some(workspace) = self.state.git_workspace_of_project(project_id, cx) {
                    workspace.update(cx, |workspace, _| {
                        let state = &mut workspace.state;
                        state.branches = branches;
                        state.tags = tags;
                        state.tracked_paths = paths;
                    });
                }
                cx.notify();
            }
            GitAppEvent::LogLoaded {
                project_id,
                request_id,
                page,
            } => {
                let Some(workspace) = self.state.git_workspace_of_project(project_id, cx) else {
                    return;
                };
                let Some((landed, keep_waiting, jump, tab_id)) =
                    workspace.update(cx, |workspace, _| {
                        let state = &mut workspace.state;
                        state.drop_request(request_id);
                        let (held, appended) = state.log.request?;
                        if held != request_id {
                            // 迟到的旧请求：界面已经发起了新一轮加载，直接丢弃。
                            log::debug!("丢弃迟到的 Git 日志结果，请求: {request_id}");
                            return None;
                        }
                        state.log.request = None;
                        state.log.loading = false;
                        state.log.error = None;
                        if appended {
                            state.log.entries.extend(page.entries.iter().cloned());
                        } else {
                            state.log.entries = page.entries.clone();
                        }
                        state.log.rows =
                            transom_git::assign_lanes(&mut state.log.graph, &state.log.entries);
                        state.log.has_more = page.has_more;
                        let jump = state.log.jump.clone();
                        let landed = jump
                            .as_ref()
                            .and_then(|target| target.resolve(&state.log.entries));
                        let within_budget =
                            state.log.jump_pages < crate::git::types::MAX_JUMP_PAGES;
                        let keep_waiting = landed.is_none()
                            && jump.is_some()
                            && state.log.has_more
                            && within_budget;
                        if landed.is_some() {
                            state.log.jump = None;
                            state.log.jump_pages = 0;
                        } else if keep_waiting {
                            state.log.jump_pages += 1;
                        }
                        Some((landed, keep_waiting, jump, workspace.tab_id))
                    })
                else {
                    return;
                };
                cx.notify();
                if let Some(index) = landed {
                    self.git_select_commit_by_index(tab_id, index, cx);
                } else if keep_waiting {
                    // 目标行还没加载：顺着分页往下翻，翻到头或超页数上限才提示找不到。
                    self.git_load_more_log(tab_id, cx);
                } else if let Some(target) = jump {
                    log::info!("Git 提交跳转未落地即停止，目标: {target:?}");
                    self.state.status = Some(rust_i18n::t!("git.log.jump_missing").to_string());
                    cx.notify();
                }
            }
            GitAppEvent::RebaseTodoLoaded {
                project_id,
                request_id,
                base,
                steps,
            } => {
                if let Some(workspace) = self.state.git_workspace_of_project(project_id, cx) {
                    workspace.update(cx, |workspace, _| {
                        workspace.state.drop_request(request_id);
                    });
                }
                self.git_apply_rebase_todo(&base, steps);
                cx.notify();
            }
            GitAppEvent::CommitUrlLoaded {
                project_id,
                request_id,
                oid,
                url,
            } => {
                if let Some(workspace) = self.state.git_workspace_of_project(project_id, cx) {
                    workspace.update(cx, |workspace, _| {
                        workspace.state.drop_request(request_id);
                    });
                }
                self.git_open_commit_url(url, &oid, cx);
            }
            GitAppEvent::CommitFilesLoaded {
                project_id,
                request_id,
                oid,
                files,
            } => {
                if let Some(workspace) = self.state.git_workspace_of_project(project_id, cx) {
                    workspace.update(cx, |workspace, _| {
                        let state = &mut workspace.state;
                        state.drop_request(request_id);
                        if state.amend_request == Some(request_id) {
                            state.amend_files = files.clone();
                            state.amend_files_loading = false;
                            state.amend_request = None;
                        } else {
                            state.commit_files = files.clone();
                            state.commit_files_loading = false;
                        }
                    });
                }
                self.git_apply_push_commit_files(&oid, files);
                cx.notify();
            }
            GitAppEvent::OutgoingCommitsLoaded {
                project_id,
                request_id,
                branch,
                remote_branch,
                commits,
            } => {
                if let Some(workspace) = self.state.git_workspace_of_project(project_id, cx) {
                    workspace.update(cx, |workspace, _| {
                        workspace.state.drop_request(request_id);
                    });
                }
                self.git_apply_outgoing_commits(project_id, &branch, &remote_branch, commits, cx);
                cx.notify();
            }
            GitAppEvent::DiffLoaded {
                project_id,
                request_id,
                diff,
            } => {
                if let Some(workspace) = self.state.git_workspace_of_project(project_id, cx) {
                    workspace.update(cx, |workspace, _| {
                        let state = &mut workspace.state;
                        state.drop_request(request_id);
                        state.apply_diff_loaded(request_id, *diff);
                    });
                }

                cx.notify();
            }
            GitAppEvent::ReadFailed {
                project_id,
                request_id,
                error,
            } => {
                let message = localized_error(&error);
                let not_repo = error.code == ErrorCode::GitNotARepo;
                if not_repo {
                    log::debug!("Git 项目尚未初始化为仓库: {project_id}");
                } else {
                    log::error!("Git 读取失败，项目: {project_id}，原因: {message}");
                }
                if let Some(workspace) = self.state.git_workspace_of_project(project_id, cx) {
                    workspace.update(cx, |workspace, _| {
                        let state = &mut workspace.state;
                        state.drop_request(request_id);
                        state.log.loading = false;
                        state.log.error = Some(message.clone());
                        if let Some(diff) = state.diff.as_mut() {
                            diff.loading = false;
                            diff.error = Some(message.clone());
                        }
                        state.commit_files_loading = false;
                        if not_repo {
                            // 不是仓库是「可以补初始化」的正常状态，不只是错误：
                            // 工作区据此显示初始化入口，而不是只剩一句红字。
                            state.not_repo = true;
                            state.status_loading = false;
                            state.status_error = Some(message.clone());
                        }
                    });
                }
                let badge = self.state.git.badges.entry(project_id).or_default();
                badge.loading = false;
                badge.error = Some(message.clone());
                if !not_repo {
                    self.set_error(message);
                }
                cx.notify();
            }
            GitAppEvent::ActionDone {
                project_id,
                action,
                error,
            } => {
                let message = error.as_ref().map(localized_error);
                match &message {
                    Some(text) => {
                        log::error!(
                            "Git 动作失败，动作: {action:?}，项目: {project_id}，原因: {text}"
                        );
                        self.git_patch_failed(project_id, action, cx);
                        self.git_rebase_failed(project_id, action, text.clone());
                        self.set_error(text.clone());
                    }
                    None => {
                        // 补丁动作要报出落盘路径，其余动作沿用动作名文案。
                        let saved = self.git_patch_done_status(project_id, cx);
                        self.state.status = Some(
                            saved.unwrap_or_else(|| rust_i18n::t!(action.i18n_key()).to_string()),
                        );
                    }
                }
                if matches!(action, GitAction::AddWorktree) && message.is_none() {
                    // 新建成功才把目录登记成项目：失败时留下一个打不开的条目只会碍事。
                    let pending = self
                        .state
                        .git_workspace_of_project(project_id, cx)
                        .and_then(|workspace| {
                            workspace
                                .update(cx, |workspace, _| workspace.state.pending_worktree.take())
                        });
                    if let Some((path, _checkout)) = pending {
                        self.add_git_project(
                            super::super::GitProjectDraft::new(std::path::PathBuf::from(&path)),
                            cx,
                        );
                    }
                }
                // 工作树的新建/删除/锁定都会改动 `git worktree list` 的结果：
                // 不重取就会把已删掉的目录继续摆在列表里，点它只会报错。
                if matches!(
                    action,
                    GitAction::AddWorktree
                        | GitAction::RemoveWorktree
                        | GitAction::LockWorktree
                        | GitAction::UnlockWorktree
                ) && message.is_none()
                {
                    let worktree_project = self
                        .state
                        .git
                        .projects
                        .iter()
                        .find(|item| item.id == project_id)
                        .cloned();
                    if let Some(project) = worktree_project {
                        self.git_reload_worktrees(&project, cx);
                    }
                }
                // 写动作后一律刷新：索引与工作区都可能被 git 顺带改写（如提交后
                // ahead 变化、worktree add 后分支增加）。
                if let Some(project) = self
                    .state
                    .git
                    .projects
                    .iter()
                    .find(|item| item.id == project_id)
                    .cloned()
                {
                    self.git_refresh(&project, cx);
                }
                if matches!(
                    action,
                    GitAction::RebaseRun | GitAction::Rebase | GitAction::RebaseAbort
                ) && message.is_none()
                {
                    // 变基改写了整段历史：日志必须重取，否则界面还停在旧提交上。
                    // 失败时不关对话框，用户要看到原因并可以改清单重试。
                    if matches!(action, GitAction::RebaseRun) {
                        self.state.modals.git.rebase = None;
                    }
                    if let Some(tab_id) = self.git_tab_for_project(project_id, cx) {
                        self.git_load_log(tab_id, 0, cx);
                    }
                }
                if matches!(action, GitAction::Push) {
                    self.git_handle_push_result(project_id, message.clone(), cx);
                }
                if matches!(
                    action,
                    GitAction::ResolveConflictOurs
                        | GitAction::ResolveConflictTheirs
                        | GitAction::ResolveConflictMark
                ) && let Some(modal) = self.state.modals.git.conflict.as_mut()
                {
                    modal.resolving = false;
                    if message.is_none() {
                        let idx = modal.selected_index;
                        if idx < modal.files.len() {
                            modal.files.remove(idx);
                        }
                        if modal.files.is_empty() {
                            self.state.modals.git.conflict = None;
                        } else {
                            modal.selected_index = modal.selected_index.min(modal.files.len() - 1);
                            modal.loading = true;
                            modal.sides = None;
                            let next_file = modal.files[modal.selected_index].clone();
                            let project = modal.project.clone();
                            let req = transom_core::GitRequestId::new();
                            self.send_command(GitCommand::LoadConflictSides {
                                project,
                                request_id: req,
                                path: next_file,
                            });
                        }
                    } else {
                        modal.error = message.clone();
                    }
                }
                if matches!(action, GitAction::Commit)
                    && let Some(tab_id) = self.git_tab_for_project(project_id, cx)
                {
                    let mut should_push = false;
                    if let Some(workspace) = self.state.git_workspace(tab_id) {
                        workspace.update(cx, |workspace, cx| {
                            let state = &mut workspace.state;
                            state.commit_submitting = false;
                            if message.is_none() {
                                state.checked = HashSet::new();
                                state.commit_amend = false;
                                state.commit_clear_pending =
                                    crate::app::app_settings(cx).git.clear_message_on_commit;
                                // 让下一次状态刷新重新选中第一个改动并拉差异。
                                state.diff = None;
                                state.last_diff = None;
                                should_push = std::mem::take(&mut state.push_after_commit);
                            }
                        });
                    }
                    if message.is_none() {
                        self.git_load_log(tab_id, 0, cx);
                        if should_push
                            && let Some(project) = self
                                .state
                                .git
                                .projects
                                .iter()
                                .find(|item| item.id == project_id)
                                .cloned()
                        {
                            let show_dialog = crate::app::app_settings(cx).git.push_dialog;
                            if show_dialog {
                                self.open_git_push_modal(&project, None, cx);
                            } else {
                                self.send_command(GitCommand::Push {
                                    project,
                                    options: transom_core::GitPushOptions::default(),
                                });
                            }
                        }
                    }
                }
                cx.notify();
            }
            GitAppEvent::WorktreesLoaded {
                project_id,
                request_id,
                entries,
            } => {
                if let Some(workspace) = self.state.git_workspace_of_project(project_id, cx) {
                    workspace.update(cx, |workspace, _| {
                        let state = &mut workspace.state;
                        state.drop_request(request_id);
                        state.worktrees = entries;
                    });
                }
                cx.notify();
            }
            GitAppEvent::StashesLoaded {
                project_id,
                request_id,
                entries,
            } => {
                if let Some(workspace) = self.state.git_workspace_of_project(project_id, cx) {
                    workspace.update(cx, |workspace, _| {
                        let state = &mut workspace.state;
                        state.drop_request(request_id);
                        state.stashes = entries;
                    });
                }
                cx.notify();
            }
            GitAppEvent::StashFilesLoaded {
                project_id,
                request_id,
                selector,
                files,
            } => {
                if let Some(workspace) = self.state.git_workspace_of_project(project_id, cx) {
                    workspace.update(cx, |workspace, _| {
                        let state = &mut workspace.state;
                        state.drop_request(request_id);
                        state.stash_files_loading = false;
                        // 迟到的清单不能写进已经换过展开项的视图：编号会随删除前移，
                        // 对不上选择器就如实丢弃，而不是让界面挂着一份别人的文件。
                        if state.expanded_stash.as_deref() == Some(selector.as_str()) {
                            state.stash_files = files;
                        }
                    });
                }
                cx.notify();
            }
            GitAppEvent::Probed { .. } => {
                self.apply_git_probe_event(event, cx);
            }
            GitAppEvent::Inited {
                request_id,
                path,
                error,
            } => {
                let ours = self
                    .state
                    .git
                    .pending_project_init
                    .is_some_and(|(pending_id, _)| pending_id == request_id);
                if ours {
                    self.apply_git_project_init_event(request_id, &path, error, cx);
                } else {
                    self.apply_git_probe_event(
                        GitAppEvent::Inited {
                            request_id,
                            path,
                            error,
                        },
                        cx,
                    );
                }
            }
            GitAppEvent::CloneProgress { .. } | GitAppEvent::CloneDone { .. } => {
                self.apply_git_clone_event(event, cx);
            }
            GitAppEvent::ConflictSidesLoaded {
                project_id,
                path,
                sides,
                ..
            } => {
                if let Some(modal) = self.state.modals.git.conflict.as_mut()
                    && modal.project.id == project_id
                    && modal.active_file().is_some_and(|f| f == path)
                {
                    modal.sides = Some(*sides);
                    modal.loading = false;
                    cx.notify();
                }
            }
        }
    }

    /// 把状态快照同时写进侧栏徽标与标签页。
    fn apply_git_status(
        &mut self,
        project_id: GitProjectId,
        status: transom_core::RepoStatus,
        cx: &mut Context<Self>,
    ) {
        let dirty = status.entries.len();
        let branch = match &status.head {
            transom_core::HeadState::OnBranch(name) => Some(name.clone()),
            transom_core::HeadState::Initial { branch } => Some(branch.clone()),
            transom_core::HeadState::Detached => status
                .oid
                .as_deref()
                .map(|oid| oid.chars().take(7).collect()),
        };
        let badge = self.state.git.badges.entry(project_id).or_default();
        badge.branch = branch;
        badge.dirty = dirty;
        badge.ahead = status.ahead;
        badge.behind = status.behind;
        badge.loading = false;
        badge.error = None;
        if let Some(workspace) = self.state.git_workspace_of_project(project_id, cx) {
            workspace.update(cx, |workspace, _| {
                let state = &mut workspace.state;
                state.status_error = None;
                state.status_loading = false;
                state.not_repo = false;
                state.status = Some(status);
            });
        }
        // 提交页首屏直接给第一个改动的差异：用户不用先点一下才知道右侧能看什么。
        if let Some(tab_id) = self.git_tab_for_project(project_id, cx) {
            let first = self
                .state
                .git_workspace(tab_id)
                .map(|workspace| {
                    let state = &workspace.read(cx).state;
                    state
                        .diff
                        .is_none()
                        .then(|| state.status.as_ref().and_then(super::first_worktree_diff))
                        .flatten()
                })
                .unwrap_or_default();
            if let Some((group, path)) = first
                && let Some(project) = self
                    .state
                    .git
                    .projects
                    .iter()
                    .find(|project| project.id == project_id)
                    .cloned()
            {
                self.git_open_worktree_diff(&project, group, path, cx);
            }
        }
        cx.notify();
    }
}
