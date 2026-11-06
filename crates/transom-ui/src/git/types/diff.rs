//! Git 差异视图显示选项与状态模型。

use transom_core::{DiffContent, GitRequestId};

/// 空白处理档位（对应 IDEA 差异工具条的「忽略差异」）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum IgnoreWhitespace {
    /// 不忽略。
    #[default]
    None,
    /// 修整空白：去掉行首行尾空白。
    Trim,
    /// 忽略所有空白。
    All,
    /// 忽略空白与空行。
    BlankLines,
}

/// 差异高亮粒度（对应 IDEA 的「高亮显示差异」）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum DiffHighlight {
    /// 整行。
    #[default]
    Line,
    /// 行内单词级高亮。
    Word,
    /// 不做任何高亮着色：IDEA 的「无」档，看结构不看颜色。
    None,
}

/// 差异视图显示选项。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GitDiffOptions {
    /// 并排两栏还是统一单栏。
    pub side_by_side: bool,
    /// 是否显示两侧行号。
    pub show_line_numbers: bool,
    /// 长行是否自动换行。
    pub wrap_lines: bool,
    /// 空白处理档位。
    pub ignore_whitespace: IgnoreWhitespace,
    /// 高亮粒度。
    pub highlight: DiffHighlight,
}

impl Default for GitDiffOptions {
    fn default() -> Self {
        Self {
            side_by_side: true,
            show_line_numbers: true,
            wrap_lines: false,
            ignore_whitespace: IgnoreWhitespace::default(),
            highlight: DiffHighlight::default(),
        }
    }
}

/// 差异面板状态。
pub(crate) struct GitDiffState {
    pub path: String,
    /// 差异来源标签：如「工作区 vs 索引」。
    pub left_label: String,
    pub right_label: String,
    pub content: Option<DiffContent>,
    pub loading: bool,
    pub error: Option<String>,
    pub request_id: GitRequestId,
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
