//! Git 面板的界面状态定义。

pub(crate) mod diff;
pub(crate) mod log;
pub(crate) mod sidebar;
pub(crate) mod workspace;

pub(crate) use diff::{
    GitDiffOptions, GitDiffRequest, GitDiffState, HunkActionCapability, first_worktree_diff,
};
pub(crate) use log::{LogColumn, LogColumns, LogJump, MAX_JUMP_PAGES};
pub(crate) use sidebar::{GitBadge, GitDropTarget, GitHub, GitSidebarDrag};
pub(crate) use workspace::{GitCommitTab, GitView, GitWorkspaceState};
