//! Git 差异视图显示选项与状态模型。

pub(crate) use crate::diff_pane::{DiffViewOptions as GitDiffOptions, IgnoreWhitespace};
use crate::diff_pane::PreparedHunk;
use gpui_kit::Entity;
use gpui_kit::component::diff::DiffState;
use transom_core::{DiffContent, GitRequestId, GitSide, RepoStatus, StatusRecord};

/// 差异面板状态。
#[derive(Clone)]
pub(crate) struct GitDiffState {
    pub path: String,
    /// 差异来源标签：如「工作区 vs 索引」。
    pub left_label: String,
    pub right_label: String,
    pub content: Option<DiffContent>,
    pub loading: bool,
    pub error: Option<String>,
    /// 后台计算并缓存的差异视图实体（避免每帧在 render 中重算 Myers）。
    pub diff_state: Option<Entity<DiffState>>,
    /// 后台 Myers 与补丁解析进行中。
    pub computing: bool,
    /// 计算代际令牌：丢弃过期线程回传的旧结果。
    pub compute_token: u64,
    /// 当前缓存产物对应的空白忽略档位。
    pub computed_ignore: Option<IgnoreWhitespace>,
    /// 差异统计（变更块数、新增行数、删除行数）。
    pub summary_hunks: usize,
    pub summary_additions: usize,
    pub summary_deletions: usize,
    /// 当前忽略空白设置隐藏了所有差异（存在原始字符变化但忽略空白后无差异）。
    pub whitespace_only: bool,
    /// 创建此差异视图时的请求描述符（含路径与两侧对比源，用于去重、刷新与块级操作）。
    pub request: Option<GitDiffRequest>,
    /// 提取出的单变更块定位与独立补丁列表。
    pub hunks: Vec<PreparedHunk>,
    /// 上次「上一处/下一处差异」导航停靠的块索引（0-based），用于跨文件边界推进。
    pub current_hunk: Option<usize>,
    /// 跨标签切换后的延迟跳转方向：`Some(true)` 跳首块，`Some(false)` 跳末块。
    pub pending_edge_jump: Option<bool>,
    pub request_id: GitRequestId,
}

impl GitDiffState {
    /// 构建处于「读取中」的初始差异状态。
    pub(crate) fn loading(request: GitDiffRequest, request_id: GitRequestId) -> Self {
        Self {
            path: request.path.clone(),
            left_label: request.left_label.clone(),
            right_label: request.right_label.clone(),
            content: None,
            loading: true,
            error: None,
            diff_state: None,
            computing: false,
            compute_token: 0,
            computed_ignore: None,
            summary_hunks: 0,
            summary_additions: 0,
            summary_deletions: 0,
            whitespace_only: false,
            request: Some(request),
            hunks: Vec::new(),
            current_hunk: None,
            pending_edge_jump: None,
            request_id,
        }
    }

    /// 评估当前差异视图是否支持块级动作（暂存块 / 还原块 / 取消暂存块）。
    pub(crate) fn hunk_capability(&self, status: Option<&RepoStatus>) -> HunkActionCapability {
        if self.hunks.is_empty() || self.content.as_ref().is_none_or(|c| c.truncated || c.binary) {
            return HunkActionCapability::None;
        }
        let Some(req) = self.request.as_ref() else {
            return HunkActionCapability::None;
        };
        let is_untracked = status.is_some_and(|st| {
            st.entries
                .iter()
                .any(|e| e.path == self.path && matches!(e.record, StatusRecord::Untracked))
        });
        if is_untracked {
            return HunkActionCapability::None;
        }
        if matches!((&req.old, &req.new), (GitSide::Index, GitSide::Worktree)) {
            HunkActionCapability::Unstaged
        } else if matches!((&req.old, &req.new), (GitSide::Commit(rev), GitSide::Index) if rev == "HEAD")
        {
            HunkActionCapability::Staged
        } else {
            HunkActionCapability::None
        }
    }
}

/// 当前差异视图允许的块级操作模式。
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum HunkActionCapability {
    /// 不可执行块级操作（如历史提交、搁置、截断文件或未跟踪文件）。
    None,
    /// 索引 vs 工作区：支持暂存变更块、还原变更块。
    Unstaged,
    /// HEAD vs 索引：支持取消暂存变更块。
    Staged,
}

/// 一次文件差异请求：比较的两侧与界面标签。
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct GitDiffRequest {
    pub path: String,
    pub old: transom_core::GitSide,
    pub new: transom_core::GitSide,
    pub left_label: String,
    pub right_label: String,
}

impl GitDiffRequest {
    /// 判断两次请求是否指向同一文件且对比两侧完全一致。
    pub(crate) fn same_target(&self, other: &Self) -> bool {
        self.path == other.path && self.old == other.old && self.new == other.new
    }
}

/// 第一个可看差异的改动：已暂存优先，其次工作区，最后未跟踪。
pub(crate) fn first_worktree_diff(
    status: &transom_core::RepoStatus,
) -> Option<(crate::git::commit_modal::ChangeGroup, String)> {
    use crate::git::commit_modal::ChangeGroup;
    use transom_core::{ChangeCode, StatusRecord};
    status.entries.iter().find_map(|entry| match entry.record {
        StatusRecord::Tracked { index, .. } if index != ChangeCode::Unmodified => {
            Some((ChangeGroup::Staged, entry.path.clone()))
        }
        StatusRecord::Conflict => Some((ChangeGroup::Staged, entry.path.clone())),
        StatusRecord::Tracked { worktree, .. } if worktree != ChangeCode::Unmodified => {
            Some((ChangeGroup::Unstaged, entry.path.clone()))
        }
        StatusRecord::Untracked => Some((ChangeGroup::Untracked, entry.path.clone())),
        _ => None,
    })
}
