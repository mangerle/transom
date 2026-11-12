//! Git 差异视图显示选项与状态模型。

pub(crate) use crate::diff_pane::{DiffViewOptions as GitDiffOptions, IgnoreWhitespace};
use gpui_kit::Entity;
use gpui_kit::component::diff::DiffState;
use transom_core::{DiffContent, GitRequestId};

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
    pub request_id: GitRequestId,
}

impl GitDiffState {
    /// 构建处于「读取中」的初始差异状态。
    pub(crate) fn loading(
        path: String,
        left_label: String,
        right_label: String,
        request_id: GitRequestId,
    ) -> Self {
        Self {
            path,
            left_label,
            right_label,
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
            request_id,
        }
    }
}

/// 一次文件差异请求：比较的两侧与界面标签。
#[derive(Clone)]
pub(crate) struct GitDiffRequest {
    pub path: String,
    pub old: transom_core::GitSide,
    pub new: transom_core::GitSide,
    pub left_label: String,
    pub right_label: String,
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
