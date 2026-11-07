//! Git 推送提交模态弹窗（Push Commits Dialog）：展示待推送分支、提交列表及选中提交的文件变更清单。
//!
//! 设计原理（Why）：
//! - 对齐 openidea / IntelliJ IDEA 标准推送对话框交互；
//! - 双栏结构：左栏展示待推送到远端的本地提交历史（Outgoing Commits），右栏展示选中提交包含的文件清单；
//! - 底部集成「推送标签」与「强制推送」安全选项，避免误推或无意覆盖远端分支；
//! - 点击「提交并推送」或在分支上右键「推送...」统一唤起本弹窗，保证全流程交互一致性。

mod render;

use crate::app::AppShell;
use crate::palette::Palette;
use gpui_kit::*;
use transom_core::{CommitEntry, CommitFile, GitCommand, GitProject, GitRequestId, HeadState};

/// Git 推送模态弹窗状态。
pub(crate) struct GitPushModal {
    pub project: GitProject,
    /// 本地待推送的分支名称。
    pub branch: String,
    /// 目标远端仓库名称（如 "origin"）。
    pub remote: String,
    /// 远端目标分支名称。
    pub remote_branch: String,
    /// 待推送到远端的提交清单（按时间由新到旧排列）。
    pub commits: Vec<CommitEntry>,
    /// 当前选中的提交哈希，用于联动在右栏加载变更文件清单。
    pub selected_oid: Option<String>,
    /// 选中提交的改动文件清单。
    pub commit_files: Vec<CommitFile>,
    /// 改动文件是否在途加载中。
    pub files_loading: bool,
    /// 是否携带所有标签推送（--tags）。
    pub push_tags: bool,
    /// 是否强制推送（--force-with-lease）。
    pub force: bool,
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
        commits: Vec<CommitEntry>,
    ) -> Self {
        let selected_oid = commits.first().map(|c| c.oid.clone());
        Self {
            project,
            branch,
            remote,
            remote_branch,
            commits,
            selected_oid,
            commit_files: Vec::new(),
            files_loading: false,
            push_tags: false,
            force: false,
            running: false,
            error: None,
        }
    }
}

impl AppShell {
    /// 唤起 Git 推送模态弹窗。
    ///
    /// 提取指定分支（若未传则默认 HEAD 分支）的 upstream 与 ahead 提交列表。
    pub(crate) fn open_git_push_modal(
        &mut self,
        project: &GitProject,
        target_branch: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let (branch_name, upstream_name, ahead_count) =
            if let Some(state) = self.git_state_of(project.id) {
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
                let ahead = state.status.as_ref().map(|s| s.ahead as usize).unwrap_or(0);
                (branch, upstream, ahead)
            } else {
                ("HEAD".to_string(), None, 0)
            };

        // 解析远端名与目标分支名（默认 origin / 本地分支名）。
        let (remote, remote_branch) = if let Some(ref up) = upstream_name {
            if let Some((r, b)) = up.split_once('/') {
                (r.to_string(), b.to_string())
            } else {
                ("origin".to_string(), up.clone())
            }
        } else {
            ("origin".to_string(), branch_name.clone())
        };

        // 获取待推送的本地提交列表。
        let commits = if let Some(state) = self.git_state_of(project.id) {
            let entries = &state.log.entries;
            if ahead_count > 0 {
                entries.iter().take(ahead_count).cloned().collect()
            } else if !entries.is_empty() {
                // 若 ahead 尚未更新或初次推送，显示最近提交供用户核验
                entries.iter().take(10).cloned().collect()
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };

        let mut modal =
            GitPushModal::new(project.clone(), branch_name, remote, remote_branch, commits);

        if let Some(first_oid) = modal.selected_oid.clone() {
            modal.files_loading = true;
            let request_id = GitRequestId::new();
            if let Some(state) = self.git_state_mut(project.id) {
                state.pending.push(request_id);
            }
            self.send_command(GitCommand::LoadCommitFiles {
                project: project.clone(),
                request_id,
                oid: first_oid,
            });
        }

        self.state.modals.git.push = Some(modal);
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

    /// 切换推送标签选项。
    pub(crate) fn git_toggle_push_tags(&mut self, cx: &mut Context<Self>) {
        if let Some(modal) = self.state.modals.git.push.as_mut() {
            modal.push_tags = !modal.push_tags;
            cx.notify();
        }
    }

    /// 切换强制推送选项。
    pub(crate) fn git_toggle_push_force(&mut self, cx: &mut Context<Self>) {
        if let Some(modal) = self.state.modals.git.push.as_mut() {
            modal.force = !modal.force;
            cx.notify();
        }
    }

    /// 执行推送动作。
    pub(crate) fn submit_git_push_modal(&mut self, cx: &mut Context<Self>) {
        let Some(modal) = self.state.modals.git.push.as_mut() else {
            return;
        };
        if modal.running {
            return;
        }
        modal.running = true;
        let project = modal.project.clone();
        let branch = modal.branch.clone();
        let tags = modal.push_tags;

        self.send_command(GitCommand::Push {
            project,
            branch: Some(branch),
            tags,
        });
        self.close_git_push_modal(cx);
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
