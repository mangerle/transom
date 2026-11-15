//! Git 推送提交模态弹窗视图实体（Push Commits Dialog）。
//!
//! 设计原理（Why）：
//! - 完整对齐 openidea / IntelliJ IDEA 标准推送对话框交互；
//! - 双栏结构：左栏展示待推送到远端的真实提交历史（Outgoing Commits），右栏展示选中提交包含的文件清单与详细说明；
//! - 实体化后选项切换、提交选择与校验均在本实体内完成，跨模块动作经由 `GitPushModalEvent` 上报外壳。

mod render;
mod views;

use crate::palette::Palette;
use gpui_kit::*;
use transom_core::{
    CommitEntry, CommitFile, GitProject, GitProjectId, GitPushOptions, PushTagsMode, TabId,
};

/// 推送模态弹窗发往外壳的意图事件。
pub(crate) enum GitPushModalEvent {
    /// 请求后台加载指定提交的改动文件清单。
    LoadCommitFiles { project: GitProject, oid: String },
    /// 提交执行远端推送。
    Submit {
        project: GitProject,
        options: GitPushOptions,
    },
    /// 打开指定提交文件的差异对比视图。
    OpenFileDiff { tab_id: TabId, path: String },
    /// 复制提交 OID 到剪贴板。
    CopyOid(String),
    /// 请求关闭弹窗。
    Close,
}

/// Git 推送模态弹窗视图实体。
pub(crate) struct GitPushModalView {
    pub project: GitProject,
    pub tab_id: Option<TabId>,
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

impl EventEmitter<GitPushModalEvent> for GitPushModalView {}

impl Render for GitPushModalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::from(cx);
        render::render_push_dialog(self, window, cx, &p)
    }
}

impl GitPushModalView {
    pub(crate) fn new(
        project: GitProject,
        tab_id: Option<TabId>,
        branch: String,
        remote: String,
        remote_branch: String,
    ) -> Self {
        Self {
            project,
            tab_id,
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

    /// 待推送提交列表加载完成落地，若存在首条提交则自动请求其改动文件。
    pub(crate) fn apply_outgoing_commits(
        &mut self,
        project_id: GitProjectId,
        branch: &str,
        remote_branch: &str,
        commits: Vec<CommitEntry>,
        cx: &mut Context<Self>,
    ) {
        if self.project.id != project_id
            || self.branch != branch
            || self.remote_branch != remote_branch
        {
            return;
        }
        self.commits_loading = false;
        self.selected_oid = commits.first().map(|c| c.oid.clone());
        let first_oid = self.selected_oid.clone();
        self.commits = commits;
        self.files_loading = first_oid.is_some();
        if let Some(oid) = first_oid {
            cx.emit(GitPushModalEvent::LoadCommitFiles {
                project: self.project.clone(),
                oid,
            });
        }
        cx.notify();
    }

    /// 提交的文件清单加载完成落地。
    pub(crate) fn apply_commit_files(
        &mut self,
        oid: &str,
        files: Vec<CommitFile>,
        cx: &mut Context<Self>,
    ) {
        if self.selected_oid.as_deref() == Some(oid) {
            self.commit_files = files;
            self.files_loading = false;
            cx.notify();
        }
    }

    /// 处理推送结果；成功时请求关闭，失败时保留弹窗并展示错误。
    pub(crate) fn handle_push_result(
        &mut self,
        project_id: GitProjectId,
        error: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if self.project.id != project_id {
            return;
        }
        match error {
            None => cx.emit(GitPushModalEvent::Close),
            Some(err) => {
                self.running = false;
                self.error = Some(err);
                cx.notify();
            }
        }
    }

    /// 切换当前选中的待推送提交，并拉取该提交的改动文件清单。
    pub(super) fn select_commit(&mut self, oid: String, cx: &mut Context<Self>) {
        if self.selected_oid.as_deref() == Some(&oid) {
            return;
        }
        self.selected_oid = Some(oid.clone());
        self.commit_files.clear();
        self.files_loading = true;
        cx.emit(GitPushModalEvent::LoadCommitFiles {
            project: self.project.clone(),
            oid,
        });
        cx.notify();
    }

    /// 循环切换推送标签模式（None -> FollowTags -> AllTags -> None）。
    pub(super) fn cycle_tags_mode(&mut self, cx: &mut Context<Self>) {
        self.tags_mode = match self.tags_mode {
            PushTagsMode::None => PushTagsMode::FollowTags,
            PushTagsMode::FollowTags => PushTagsMode::AllTags,
            PushTagsMode::AllTags => PushTagsMode::None,
        };
        cx.notify();
    }

    /// 切换强制推送选项（受保护分支禁止开启）。
    pub(super) fn toggle_force(&mut self, cx: &mut Context<Self>) {
        if is_git_branch_protected(&self.remote_branch, cx) {
            return;
        }
        self.force = !self.force;
        cx.notify();
    }

    /// 切换跳过预推送 Git 钩子选项。
    pub(super) fn toggle_no_verify(&mut self, cx: &mut Context<Self>) {
        self.no_verify = !self.no_verify;
        cx.notify();
    }

    /// 执行推送动作。
    pub(crate) fn submit(&mut self, cx: &mut Context<Self>) {
        if self.running {
            return;
        }
        if self.force && is_git_branch_protected(&self.remote_branch, cx) {
            self.error = Some(rust_i18n::t!("git.push.protected_branch_warning").to_string());
            cx.notify();
            return;
        }
        self.running = true;
        self.error = None;
        let options = GitPushOptions {
            remote: Some(self.remote.clone()),
            branch: Some(self.branch.clone()),
            remote_branch: Some(self.remote_branch.clone()),
            force_with_lease: self.force,
            tags: self.tags_mode,
            no_verify: self.no_verify,
        };
        cx.emit(GitPushModalEvent::Submit {
            project: self.project.clone(),
            options,
        });
        cx.notify();
    }
}

/// 判断目标分支是否属于受保护分支（禁止直接无条件强推）。
pub(super) fn is_git_branch_protected(branch: &str, cx: &App) -> bool {
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
