//! Git 推送提交模态弹窗（Push Commits Dialog）：展示待推送分支、提交列表及选中提交的文件变更清单。
//!
//! 设计原理（Why）：
//! - 完整对齐 openidea / IntelliJ IDEA 标准推送对话框交互；
//! - 双栏结构：左栏展示待推送到远端的真实提交历史（Outgoing Commits），右栏展示选中提交包含的文件清单与详细说明；
//! - 底部集成「推送标签（--follow-tags / --tags）」、「强制推送（--force-with-lease）」与「跳过钩子（--no-verify）」安全选项；
//! - 点击「提交并推送」或在分支上右键「推送...」统一唤起本弹窗，保证全流程交互一致性。

mod render;
mod views;

use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::*;
use transom_core::{
    CommitEntry, CommitFile, GitCommand, GitProject, GitProjectId, GitPushOptions, GitRequestId,
    HeadState, PushTagsMode,
};

/// Git 推送模态弹窗状态。
pub(crate) struct GitPushModal {
    pub project: GitProject,
    /// 本地待推送的分支名称。
    pub branch: String,
    /// 目标远端仓库名称（如 "origin"）。
    pub remote: String,
    /// 远端目标分支名称。
    pub remote_branch: String,
    /// 待推送到远端的真实提交清单（按时间由新到旧排列）。
    pub commits: Vec<CommitEntry>,
    /// 待推送提交是否正在后台计算加载中。
    pub commits_loading: bool,
    /// 当前选中的提交哈希，用于联动在右栏加载变更文件清单。
    pub selected_oid: Option<String>,
    /// 选中提交的改动文件清单。
    pub commit_files: Vec<CommitFile>,
    /// 改动文件是否在途加载中。
    pub files_loading: bool,
    /// 标签推送模式（None / FollowTags / AllTags）。
    pub tags_mode: PushTagsMode,
    /// 是否强制推送（--force-with-lease）。
    pub force: bool,
    /// 是否跳过预推送 Git 钩子（--no-verify）。
    pub no_verify: bool,
    /// 是否正在向远端推送中。
    pub running: bool,
    /// 错误信息（推送被拒绝、冲突或网络异常）。
    pub error: Option<String>,
}

impl GitPushModal {
    pub(crate) fn new(
        project: GitProject,
        branch: String,
        remote: String,
        remote_branch: String,
    ) -> Self {
        Self {
            project,
            branch,
            remote,
            remote_branch,
            commits: Vec::new(),
            commits_loading: true,
            selected_oid: None,
            commit_files: Vec::new(),
            files_loading: false,
            tags_mode: PushTagsMode::FollowTags,
            force: false,
            no_verify: false,
            running: false,
            error: None,
        }
    }
}

impl AppShell {
    /// 判断目标分支是否属于受保护分支（禁止直接无条件强推）。
    pub(crate) fn is_git_branch_protected(&self, branch: &str, cx: &App) -> bool {
        let target = branch.trim().to_lowercase();
        let settings = crate::app::app_settings(cx);
        let raw = settings.git.protected_branches.trim();
        let patterns: Vec<&str> = if raw.is_empty() {
            vec!["main", "master", "release", "prod"]
        } else {
            raw.split([',', '，', ';', ' '])
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .collect()
        };
        patterns.iter().any(|pat| {
            let p = pat.to_lowercase();
            !p.is_empty() && (target == p || target.starts_with(&format!("{p}/")))
        })
    }

    /// 唤起 Git 推送模态弹窗并异步加载真实的待推送提交。
    pub(crate) fn open_git_push_modal(
        &mut self,
        project: &GitProject,
        target_branch: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let (branch_name, upstream_name) = if let Some(state) = self.git_state_of(project.id) {
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
        } else {
            ("HEAD".to_string(), None)
        };

        let (remote, remote_branch) = if let Some(ref up) = upstream_name {
            if let Some((r, b)) = up.split_once('/') {
                (r.to_string(), b.to_string())
            } else {
                ("origin".to_string(), up.clone())
            }
        } else {
            ("origin".to_string(), branch_name.clone())
        };

        let modal = GitPushModal::new(
            project.clone(),
            branch_name.clone(),
            remote.clone(),
            remote_branch.clone(),
        );

        // 派发 LoadOutgoingCommits 查询真实的待推送提交历史
        let request_id = GitRequestId::new();
        if let Some(state) = self.git_state_mut(project.id) {
            state.pending.push(request_id);
        }
        self.send_command(GitCommand::LoadOutgoingCommits {
            project: project.clone(),
            request_id,
            branch: branch_name,
            remote: Some(remote),
            remote_branch: Some(remote_branch),
        });

        self.state.modals.git.push = Some(modal);
        cx.notify();
    }

    /// 待推送提交列表加载完成落地。
    pub(crate) fn git_apply_outgoing_commits(
        &mut self,
        project_id: GitProjectId,
        branch: &str,
        remote_branch: &str,
        commits: Vec<CommitEntry>,
        cx: &mut Context<Self>,
    ) {
        let (project, first_oid) = {
            let Some(modal) = self.state.modals.git.push.as_mut() else {
                return;
            };
            if modal.project.id != project_id
                || modal.branch != branch
                || modal.remote_branch != remote_branch
            {
                return;
            }

            modal.commits_loading = false;
            modal.selected_oid = commits.first().map(|c| c.oid.clone());
            let first_oid = modal.selected_oid.clone();
            modal.commits = commits;
            modal.files_loading = first_oid.is_some();
            (modal.project.clone(), first_oid)
        };

        if let Some(oid) = first_oid {
            let request_id = GitRequestId::new();
            if let Some(state) = self.git_state_mut(project_id) {
                state.pending.push(request_id);
            }
            self.send_command(GitCommand::LoadCommitFiles {
                project,
                request_id,
                oid,
            });
        }
        cx.notify();
    }

    /// 关闭推送模态弹窗。
    pub(crate) fn close_git_push_modal(&mut self, cx: &mut Context<Self>) {
        self.state.modals.git.push = None;
        cx.notify();
    }

    /// 切换当前选中的待推送提交，并拉取该提交的改动文件清单。
    pub(crate) fn git_select_push_commit(&mut self, oid: String, cx: &mut Context<Self>) {
        let Some(modal) = self.state.modals.git.push.as_mut() else {
            return;
        };
        if modal.selected_oid.as_deref() == Some(&oid) {
            return;
        }
        modal.selected_oid = Some(oid.clone());
        modal.commit_files.clear();
        modal.files_loading = true;

        let project = modal.project.clone();
        let request_id = GitRequestId::new();
        if let Some(state) = self.git_state_mut(project.id) {
            state.pending.push(request_id);
        }
        self.send_command(GitCommand::LoadCommitFiles {
            project,
            request_id,
            oid,
        });
        cx.notify();
    }

    /// 提交的文件清单加载完成落地。
    pub(crate) fn git_apply_push_commit_files(&mut self, oid: &str, files: Vec<CommitFile>) {
        let Some(modal) = self.state.modals.git.push.as_mut() else {
            return;
        };
        if modal.selected_oid.as_deref() == Some(oid) {
            modal.commit_files = files;
            modal.files_loading = false;
        }
    }

    /// 循环切换推送标签模式（None -> FollowTags -> AllTags -> None）。
    pub(crate) fn git_cycle_push_tags_mode(&mut self, cx: &mut Context<Self>) {
        if let Some(modal) = self.state.modals.git.push.as_mut() {
            modal.tags_mode = match modal.tags_mode {
                PushTagsMode::None => PushTagsMode::FollowTags,
                PushTagsMode::FollowTags => PushTagsMode::AllTags,
                PushTagsMode::AllTags => PushTagsMode::None,
            };
            cx.notify();
        }
    }

    /// 切换强制推送选项（受保护分支禁止开启）。
    pub(crate) fn git_toggle_push_force(&mut self, cx: &mut Context<Self>) {
        let is_protected = self
            .state
            .modals
            .git
            .push
            .as_ref()
            .map(|m| self.is_git_branch_protected(&m.remote_branch, cx))
            .unwrap_or(false);
        if is_protected {
            return;
        }
        if let Some(modal) = self.state.modals.git.push.as_mut() {
            modal.force = !modal.force;
            cx.notify();
        }
    }

    /// 切换跳过预推送 Git 钩子选项。
    pub(crate) fn git_toggle_push_no_verify(&mut self, cx: &mut Context<Self>) {
        if let Some(modal) = self.state.modals.git.push.as_mut() {
            modal.no_verify = !modal.no_verify;
            cx.notify();
        }
    }

    /// 执行推送动作。
    pub(crate) fn submit_git_push_modal(&mut self, cx: &mut Context<Self>) {
        let Some(modal) = self.state.modals.git.push.as_ref() else {
            return;
        };
        if modal.running {
            return;
        }
        let force = modal.force;
        let remote_branch = modal.remote_branch.clone();
        if force && self.is_git_branch_protected(&remote_branch, cx) {
            if let Some(m) = self.state.modals.git.push.as_mut() {
                m.error = Some(rust_i18n::t!("git.push.protected_branch_warning").to_string());
            }
            cx.notify();
            return;
        }

        let Some(modal) = self.state.modals.git.push.as_mut() else {
            return;
        };
        modal.running = true;
        modal.error = None;
        let project = modal.project.clone();
        let options = GitPushOptions {
            remote: Some(modal.remote.clone()),
            branch: Some(modal.branch.clone()),
            remote_branch: Some(modal.remote_branch.clone()),
            force_with_lease: modal.force,
            tags: modal.tags_mode,
            no_verify: modal.no_verify,
        };

        self.send_command(GitCommand::Push { project, options });
        cx.notify();
    }

    /// 处理推送结果（由 ActionDone 统一收口回调）。
    pub(crate) fn git_handle_push_result(
        &mut self,
        project_id: GitProjectId,
        error: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(modal) = self.state.modals.git.push.as_mut() else {
            return;
        };
        if modal.project.id != project_id {
            return;
        }

        match error {
            None => {
                // 推送成功，关闭模态弹窗并给出成功提示
                self.close_git_push_modal(cx);
            }
            Some(err) => {
                // 推送失败，保留现场并将错误回显在顶部错误横幅
                modal.running = false;
                modal.error = Some(err);
                cx.notify();
            }
        }
    }

    /// 渲染推送模态弹窗主体（双栏结构）。
    pub(crate) fn render_git_push_dialog_body(
        &self,
        app: &Entity<AppShell>,
        window: &mut Window,
        cx: &App,
        p: &Palette,
    ) -> AnyElement {
        render::render_push_dialog(self, app, window, cx, p)
    }
}
