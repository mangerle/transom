//! Git 操作动作与参数模型。

use std::path::PathBuf;

use super::status::ChangeCode;

/// 界面需要区分结果的写操作类别。
///
/// 设计原理（Why）：控制器完成一个动作后只回一条 `ActionDone`，界面按类别
/// 决定是刷新（暂存/提交）还是保持原状（拉取失败要弹错），不必为每个动作
/// 定义一个事件变体。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitAction {
    Stage,
    StageAll,
    Unstage,
    Discard,
    DeleteUntracked,
    Commit,
    Checkout,
    CreateBranch,
    RenameBranch,
    DeleteBranch,
    CreateTag,
    DeleteTag,
    AddWorktree,
    Fetch,
    Push,
    PushUpTo,
    Pull,
    Init,
    Reset,
    Revert,
    CherryPick,
    Merge,
    Rebase,
    RebaseRun,
    RebaseAbort,
    RemoveWorktree,
    LockWorktree,
    UnlockWorktree,
    SquashParent,
    FixupParent,
    StashSave,
    StashApply,
    StashPop,
    StashDrop,
    CreatePatch,
    ResolveConflictOurs,
    ResolveConflictTheirs,
    ResolveConflictMark,
}

impl GitAction {
    /// 动作的国际化文案键，供通知与确认框复用。
    pub fn i18n_key(self) -> &'static str {
        match self {
            Self::Stage => "git.action.stage",
            Self::StageAll => "git.action.stage_all",
            Self::Unstage => "git.action.unstage",
            Self::Discard => "git.action.discard",
            Self::DeleteUntracked => "git.action.delete_untracked",
            Self::Commit => "git.action.commit",
            Self::Checkout => "git.action.checkout",
            Self::CreateBranch => "git.action.create_branch",
            Self::RenameBranch => "git.action.rename_branch",
            Self::DeleteBranch => "git.action.delete_branch",
            Self::CreateTag => "git.action.create_tag",
            Self::DeleteTag => "git.action.delete_tag",
            Self::AddWorktree => "git.action.add_worktree",
            Self::Fetch => "git.action.fetch",
            Self::Push => "git.action.push",
            Self::PushUpTo => "git.action.push_up_to",
            Self::Pull => "git.action.pull",
            Self::Init => "git.action.init",
            Self::Reset => "git.action.reset",
            Self::Revert => "git.action.revert",
            Self::CherryPick => "git.action.cherry_pick",
            Self::Merge => "git.action.merge",
            Self::Rebase => "git.action.rebase",
            Self::RebaseRun => "git.action.rebase_run",
            Self::RebaseAbort => "git.action.rebase_abort",
            Self::RemoveWorktree => "git.action.remove_worktree",
            Self::LockWorktree => "git.action.lock_worktree",
            Self::UnlockWorktree => "git.action.unlock_worktree",
            Self::SquashParent => "git.action.squash_parent",
            Self::FixupParent => "git.action.fixup_parent",
            Self::StashSave => "git.action.stash_save",
            Self::StashApply => "git.action.stash_apply",
            Self::StashPop => "git.action.stash_pop",
            Self::StashDrop => "git.action.stash_drop",
            Self::CreatePatch => "git.action.create_patch",
            Self::ResolveConflictOurs => "git.action.resolve_ours",
            Self::ResolveConflictTheirs => "git.action.resolve_theirs",
            Self::ResolveConflictMark => "git.action.resolve_mark",
        }
    }
}

/// 交互式变基里一行的动作。
///
/// 设计原理（Why）：不做 `edit`。`edit` 会把变基停在中间状态，需要「继续/中止」
/// 的现场控制与冲突处理界面，那是另一条能力；在这里放出 `edit` 只会把用户
/// 带进一个界面处理不完的半完成状态。其余四种都能一次跑完。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitRebaseAction {
    Pick,
    Reword,
    Squash,
    Fixup,
    Drop,
}

impl GitRebaseAction {
    /// git 待办清单里的关键字。
    pub fn keyword(self) -> &'static str {
        match self {
            Self::Pick => "pick",
            Self::Reword => "reword",
            Self::Squash => "squash",
            Self::Fixup => "fixup",
            Self::Drop => "drop",
        }
    }

    /// 界面文案键。
    pub fn i18n_key(self) -> &'static str {
        match self {
            Self::Pick => "git.rebase.pick",
            Self::Reword => "git.rebase.reword",
            Self::Squash => "git.rebase.squash",
            Self::Fixup => "git.rebase.fixup",
            Self::Drop => "git.rebase.drop",
        }
    }

    /// 循环切换下一个动作：与 IDEA 里连点动作格的行为一致。
    pub fn next(self) -> Self {
        match self {
            Self::Pick => Self::Squash,
            Self::Squash => Self::Fixup,
            Self::Fixup => Self::Drop,
            Self::Drop => Self::Reword,
            Self::Reword => Self::Pick,
        }
    }
}

/// 待办清单的一行：动作 + 提交 + 该提交的标题。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitRebaseStep {
    pub action: GitRebaseAction,
    pub oid: String,
    pub subject: String,
}

/// 重置力度（对应 IDEA「将当前分支重置到此处」的三档）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitResetKind {
    /// 只移动 HEAD，索引与工作区都不动。
    Soft,
    /// 移动 HEAD 并重置索引，保留工作区改动。
    Mixed,
    /// 移动 HEAD、重置索引并丢弃工作区改动：破坏性，界面必须已确认。
    Hard,
}

impl GitResetKind {
    /// 档位文案键。
    pub fn i18n_key(self) -> &'static str {
        match self {
            Self::Soft => "git.reset.soft",
            Self::Mixed => "git.reset.mixed",
            Self::Hard => "git.reset.hard",
        }
    }
}

/// 检出目标：具名分支或游离到某个提交。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitCheckout {
    Branch(String),
    Detached(String),
}

/// 一条搁置（stash）记录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StashEntry {
    /// `stash@{0}` 形态的选择器，可直接传给后续子命令。
    pub selector: String,
    /// git 自动生成的标题（`WIP on main: ...`）。
    pub subject: String,
    /// 用户填写的说明；为空时与标题相同。
    pub message: String,
}

impl StashEntry {
    /// 界面展示名：优先用户说明。
    pub fn display(&self) -> &str {
        if self.message.trim().is_empty() {
            &self.subject
        } else {
            &self.message
        }
    }
}

/// 搁置项里的一条文件。
///
/// 设计原理（Why）：`untracked` 必须显式带出来。搁置清单是
/// `git stash show --include-untracked` 把两份内容合并后的结果——跟踪部分的
/// 改动住在 `stash@{n}` 自身的树里，而收进搁置的未跟踪文件只存在于第三个父提交
/// `stash@{n}^3`。差异视图要按条选对侧，靠状态字母猜会把「先 git add 再搁置的
/// 新文件」错认成未跟踪，界面就显示一份空差异。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StashFile {
    /// 搁置后的路径。
    pub path: String,
    /// 重命名/复制时的原路径。
    pub old_path: Option<String>,
    /// 状态字母。
    pub change: ChangeCode,
    /// 内容是否在 `^3`（未跟踪侧）。
    pub untracked: bool,
}

/// 一个工作树（`git worktree`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeEntry {
    /// 工作树目录；现存路径会规范化为宿主机的绝对路径表示。
    pub path: PathBuf,
    /// HEAD 指向的提交号。
    pub head: String,
    /// 检出的分支；游离头指针时为 `None`。
    pub branch: Option<String>,
    /// 是否游离头指针。
    pub detached: bool,
    /// 是否为裸工作树（主仓库本身）。
    pub bare: bool,
    /// 是否被 `git worktree lock` 锁住。
    pub locked: bool,
    /// 锁定原因：`git worktree lock --reason` 写的文本，未带原因时为 `None`。
    ///
    /// 界面必须显示它——锁住的工作树删不掉，git 报的原文里就带这个原因，
    /// 不给看就等于让用户对着一句「无法删除已锁定的工作区」猜。
    pub lock_reason: Option<String>,
    /// 可清理原因：目录已不存在或重复登记等，`None` 表示正常。
    ///
    /// 与 `locked` 一样来自 porcelain 输出，git 给的文本原样带上来，
    /// 界面用来解释「为什么这一行是灰的、清理会发生什么」。
    pub prune_reason: Option<String>,
}

/// 推送附带标签的模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum PushTagsMode {
    #[default]
    None,
    /// 仅推送带注释且可达的关联标签（git push --follow-tags）。
    FollowTags,
    /// 推送所有本地标签（git push --tags）。
    AllTags,
}

/// 高级 Git 推送参数。
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GitPushOptions {
    /// 目标远端名称，如 "origin"
    pub remote: Option<String>,
    /// 本地分支名称
    pub branch: Option<String>,
    /// 目标远端分支名称（如与本地分支不一致则通过 refspec 推送）
    pub remote_branch: Option<String>,
    /// 是否启用安全租约强制推送（--force-with-lease）
    pub force_with_lease: bool,
    /// 标签推送模式
    pub tags: PushTagsMode,
    /// 是否跳过预推送 Git 钩子（--no-verify）
    pub no_verify: bool,
}

/// 离线搁置项（Shelf Item）：独立于 Git stash 栈的本地补丁集合。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ShelfEntry {
    /// 搁置项唯一标识。
    pub id: String,
    /// 搁置项标题/说明。
    pub name: String,
    /// 搁置时所在的分支名称。
    pub branch: String,
    /// 创建时间字符串。
    pub created_at: String,
    /// 补丁包含的文件路径清单。
    pub files: Vec<String>,
    /// 补丁文本（unified diff 格式）。
    pub patch: String,
}

/// 冲突文件的三方内容视图。
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct ConflictSides {
    /// 共同祖先版本（Stage 1，如有）。
    pub base: Option<String>,
    /// 本地/当前分支版本（Stage 2: Ours）。
    pub ours: Option<String>,
    /// 传入分支版本（Stage 3: Theirs）。
    pub theirs: Option<String>,
    /// 当前工作区文件内容（包含冲突标记或用户已编辑的内容）。
    pub working: String,
}
