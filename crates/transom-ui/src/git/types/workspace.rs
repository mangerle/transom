//! Git 工作区标签页状态模型。

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use gpui_kit::component::input::{InputState, TextareaState};
use gpui_kit::component::resizable::ResizableState;
use gpui_kit::{AppContext, Context, Entity, Window};
use transom_core::{
    BranchEntry, CommitEntry, CommitFile, DiffContent, GitProject, GitRequestId, RepoStatus,
    TagEntry,
};

use super::diff::{GitDiffOptions, GitDiffState};
use super::log::GitLogState;

/// 标签页主体显示哪一屏。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum GitView {
    /// 概览：内嵌提交页（改动树 + 提交消息 + 右侧 diff）。
    #[default]
    Overview,
    /// 日志工作区：入口按钮本轮先移除（用户尚未决定放哪），代码保留待接回。
    #[allow(dead_code)]
    Log,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum GitCommitTab {
    /// 提交：改动树 + 消息 + 提交按钮。
    #[default]
    Commit,
    /// 搁置：内联的搁置列表，与提交页签在左栏内切换。
    Shelf,
}

/// 一个仓库标签页的全部状态。
pub(crate) struct GitWorkspaceState {
    pub project: GitProject,
    /// 顶部「操作」按钮切换的屏。
    pub view: GitView,
    pub status: Option<RepoStatus>,
    pub status_loading: bool,
    pub status_error: Option<String>,
    /// 目录当前不是 Git 仓库：以「未初始化」状态加入的项目，或仓库被外部删除。
    /// 工作区据此显示初始化入口，而不是只留一句错误。
    pub not_repo: bool,
    /// 面板里正在补 `git init`：按钮置灰，避免重复派发。
    pub init_running: bool,
    pub branches: Vec<BranchEntry>,
    pub tags: Vec<TagEntry>,
    pub log: GitLogState,
    /// 日志表里选中的提交下标。
    pub selected_commit: Option<usize>,
    /// 选中提交的文件清单。
    pub commit_files: Vec<CommitFile>,
    pub commit_files_loading: bool,
    /// 当前差异（右栏）。
    pub diff: Option<GitDiffState>,
    /// 左侧引用栏是否展开。
    pub refs_panel: bool,
    /// 引用栏各分组的展开态（本地/分支/远程/标签）。
    pub refs_expanded: HashMap<String, bool>,
    /// 概览里勾选的文件路径（提交面板的待提交集）。
    pub checked: HashSet<String>,
    /// 左栏宽度（引用树）。
    pub refs_width: f32,
    /// 右栏宽度（详情 + 差异）。
    pub details_width: f32,
    /// 在途请求：视图销毁或仓库切换时统一取消。
    pub pending: Vec<GitRequestId>,
    /// 改动树里被折叠的分组键（`staged` / `changes` / `unversioned`）。
    pub collapsed_groups: HashSet<String>,
    /// 改动树是否按目录折叠：IDEA 的「分组依据 = 目录」形态。
    pub group_by_directory: bool,
    /// 按目录模式下被折叠的目录（键为 `组/目录`，避免不同组互相影响）。
    pub collapsed_dirs: HashSet<String>,
    /// 日志过滤条件草稿：改完点回车或下拉才发请求，避免每敲一个字跑一次子进程。
    pub query: transom_core::LogQuery,
    /// 差异视图的显示选项。
    pub diff_options: GitDiffOptions,
    /// 最近一次差异请求：选项变化时按原参数重发，而不是让用户重新点文件。
    pub last_diff: Option<crate::git::actions::GitDiffRequest>,
    /// 日志检索框（文本或哈希）。
    pub search_input: Entity<InputState>,
    /// 日志作者过滤框。
    pub author_input: Entity<InputState>,
    /// 跟踪文件清单（路径过滤下拉用，随 LoadRefs 一起取）。
    pub tracked_paths: Vec<String>,
    /// 工作树列表。
    pub worktrees: Vec<transom_core::WorktreeEntry>,
    /// 在途的新建工作树：`(目录, 创建后是否顺带登记进列表)`。
    ///
    /// 目前恒为登记：`git worktree add` 成功即说明目录可用，列表里多一份条目
    /// 是用户点「创建」时预期的结果，不再另设开关。
    pub pending_worktree: Option<(String, bool)>,
    /// 搁置项列表。
    pub stashes: Vec<transom_core::StashEntry>,
    /// 展开的搁置项选择器；同一时刻只展开一条，与 IDEA 的搁置树一致。
    pub expanded_stash: Option<String>,
    /// 展开项的文件清单：只存当前展开这一条，收起即清空，避免整栈常驻内存。
    pub stash_files: Vec<transom_core::StashFile>,
    /// 清单是否在读：读回前不给第二次子进程，防止连点展开堆请求。
    pub stash_files_loading: bool,
    /// 搁置页 / 提交页的当前页签。
    pub commit_tab: GitCommitTab,
    /// 本次「创建补丁」的目标路径：对话框选完就记下来，写成功时用它报落盘位置。
    pub patch_target: Option<PathBuf>,
    /// 提交面板的提交消息（随标签页存活，切到日志再切回来不丢草稿）。
    pub commit_message: Entity<TextareaState>,
    /// 「修正上次提交」。
    pub commit_amend: bool,
    /// 提交请求在途：两个提交按钮都置灰。
    pub commit_submitting: bool,
    /// 提交成功后置位：提交页拿到 Window 时再清空消息框。
    pub commit_clear_pending: bool,
    /// 提交页左右分栏的拖拽状态：切到日志再回来保留用户拖出的宽度。
    pub commit_split: Entity<ResizableState>,
    /// 「修正」目标提交的文件清单（只读展示）。
    pub amend_files: Vec<CommitFile>,
    pub amend_files_loading: bool,
    /// 在途的修正文件请求：事件按它区分提交文件的两种用途。
    pub amend_request: Option<GitRequestId>,
    /// 「修正」目标提交的 OID（None 表示 HEAD，Some(oid) 表示选中的历史未推送提交）。
    pub amend_target: Option<String>,
    /// 已打开的文件差异标签列表（「一个标签一个名字」）。
    pub diff_tabs: Vec<GitDiffState>,
    /// 当前激活的文件差异标签索引。
    pub active_diff_tab: Option<usize>,
}

impl GitWorkspaceState {
    pub(crate) fn new(
        project: GitProject,
        window: &mut Window,
        cx: &mut Context<crate::app::AppShell>,
    ) -> Self {
        let search_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("git.filter.search_placeholder").to_string())
        });
        let author_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(rust_i18n::t!("git.filter.author_placeholder").to_string())
        });
        let commit_message = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder(rust_i18n::t!("git.commit.message_placeholder").to_string())
        });
        let commit_split = cx.new(|_| ResizableState::default());
        Self {
            project,
            view: GitView::default(),
            status: None,
            status_loading: false,
            status_error: None,
            not_repo: false,
            init_running: false,
            branches: Vec::new(),
            tags: Vec::new(),
            log: GitLogState::default(),
            selected_commit: None,
            commit_files: Vec::new(),
            commit_files_loading: false,
            diff: None,
            refs_panel: true,
            refs_expanded: HashMap::new(),
            checked: HashSet::new(),
            refs_width: 220.0,
            details_width: 460.0,
            pending: Vec::new(),
            collapsed_groups: HashSet::new(),
            group_by_directory: false,
            collapsed_dirs: HashSet::new(),
            query: transom_core::LogQuery::default(),
            diff_options: GitDiffOptions::default(),
            last_diff: None,
            search_input,
            author_input,
            tracked_paths: Vec::new(),
            worktrees: Vec::new(),
            pending_worktree: None,
            stashes: Vec::new(),
            expanded_stash: None,
            stash_files: Vec::new(),
            stash_files_loading: false,
            commit_tab: GitCommitTab::default(),
            patch_target: None,
            commit_message,
            commit_amend: false,
            commit_submitting: false,
            commit_clear_pending: false,
            commit_split,
            amend_files: Vec::new(),
            amend_files_loading: false,
            amend_request: None,
            amend_target: None,
            diff_tabs: Vec::new(),
            active_diff_tab: None,
        }
    }

    /// 获取当前激活的差异状态：优先多标签，标签为空时降级到单一 `diff`。
    pub(crate) fn active_diff(&self) -> Option<&GitDiffState> {
        self.active_diff_tab
            .and_then(|idx| self.diff_tabs.get(idx))
            .or(self.diff.as_ref())
    }

    /// 当前选中的提交。
    pub(crate) fn selected_entry(&self) -> Option<&CommitEntry> {
        self.selected_commit
            .and_then(|index| self.log.entries.get(index))
    }

    pub(crate) fn drop_request(&mut self, request_id: transom_core::GitRequestId) {
        self.pending.retain(|item| *item != request_id);
    }

    /// 将读取到的差异内容填入主 diff 与对应多标签项。
    pub(crate) fn apply_diff_loaded(&mut self, request_id: GitRequestId, content: DiffContent) {
        if let Some(current) = self.diff.as_mut()
            && current.request_id == request_id
        {
            current.content = Some(content.clone());
            current.loading = false;
        }
        for tab in &mut self.diff_tabs {
            if tab.request_id == request_id {
                tab.content = Some(content);
                tab.loading = false;
                break;
            }
        }
    }
}
