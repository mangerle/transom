//! Transom 的 Git 接入层：以系统 `git` 子进程为唯一事实来源。
//!
//! 设计原理（Why）：
//! - **为什么调系统 git 而不是 libgit2**：界面要给出的每一个动作（工作树、
//!   变基、拣选、钩子、凭据助手、子模块）都必须与用户命令行里的 git 结果
//!   完全一致。libgit2 在 worktree、部分 rebase 与凭据路径上与 git 本体存在
//!   行为差异，一旦差异出现，用户在本工具里做的操作就无法在命令行复现排查；
//!   代价是必须解析文本输出，因此所有子命令都走 porcelain v2 / `-z` /
//!   自定义控制字符分隔，并统一设 `LC_ALL=C` 锁死文案语言；
//! - **为什么本层不含任何 UI 与调度**：与 `transom-container` 同构，控制器
//!   只依赖这里的强类型快照，界面永不接触 git 原始输出，方便日后替换实现；
//! - **为什么所有查询都是无状态的一次子进程**：桌面端仓库数量有限、刷新频率
//!   由界面节流决定，常驻 git 进程（`git cat-file --batch`）能省下进程启动
//!   开销，但会引入进程生命周期与并发写入的复杂度，当前规模下不值得。
//!
//! 代价与局限：
//! - 依赖运行环境安装 git，未安装时功能整体不可用（以 [`GitError::Spawn`] 上报）；
//! - 非 UTF-8 文件名以替换字符呈现，对其发起的写操作会失败并如实报错。

mod clone;
mod diff;
mod error;
mod fields;
mod graph;
mod history;
mod log;
mod ops;
mod patch;
mod process;
mod rebase;
mod refs;
mod remote;
mod repo;
mod service;
mod stash;
mod status;
mod types;
mod web;
mod worktree;

pub use clone::clone_with_progress;
pub use diff::MAX_SIDE_BYTES;
pub use diff::Side;
pub use error::GitError;
pub use graph::{GraphRow, GraphState, assign_lanes};
pub use history::ResetKind;
pub use patch::MAX_PATCH_BYTES;
pub use repo::Repo;
pub use repo::{init, probe};
pub use service::GitService;
pub use transom_core::GitSide;
pub use types::{
    BranchEntry, ChangeCode, CommitEntry, CommitFile, DiffContent, HeadState, LogPage, LogQuery,
    LogScope, RefDecor, RefKind, RepoInfo, RepoStatus, StatusEntry, StatusRecord, TagEntry,
};

/// [`Repo`] 之上的统一门面：把各子模块的能力收敛成按仓库调用的方法。
///
/// 每个方法都是一次子进程调用，无内部缓存；调用方（控制器）负责节流与
/// 任务取消，这样界面关闭后不会遗留任何在途 git 进程。
impl Repo {
    /// 工作区状态快照；`include_ignored` 为真时一并列出被忽略的文件。
    pub async fn status(&self, include_ignored: bool) -> Result<RepoStatus, GitError> {
        status::current(self.root(), include_ignored).await
    }

    /// 分页查询提交历史。
    pub async fn log(&self, query: &LogQuery) -> Result<LogPage, GitError> {
        log::query(self.root(), query).await
    }

    /// 单个提交改动的文件清单。
    pub async fn commit_files(&self, oid: &str) -> Result<Vec<CommitFile>, GitError> {
        log::commit_files(self.root(), oid).await
    }

    /// 跟踪文件清单，供日志的路径过滤器使用。
    pub async fn tracked_files(&self) -> Result<Vec<String>, GitError> {
        log::tracked_files(self.root()).await
    }

    /// 本地与远程分支列表。
    pub async fn branches(&self) -> Result<Vec<BranchEntry>, GitError> {
        refs::branches(self.root()).await
    }

    /// 标签列表。
    pub async fn tags(&self) -> Result<Vec<TagEntry>, GitError> {
        refs::tags(self.root()).await
    }

    /// 把一个修订解析为完整提交号；解析不到返回 `None`。
    pub async fn resolve(&self, rev: &str) -> Result<Option<String>, GitError> {
        repo::resolve(self.root(), rev).await
    }

    /// 读取某文件在两个版本之间的文本差异。
    pub async fn diff(&self, old: &Side, new: &Side, path: &str) -> Result<DiffContent, GitError> {
        diff::content(self.root(), old, new, path).await
    }

    /// 暂存指定路径。
    pub async fn stage(&self, paths: &[String]) -> Result<(), GitError> {
        ops::stage(self.root(), paths).await
    }

    /// 暂存全部改动。
    pub async fn stage_all(&self) -> Result<(), GitError> {
        ops::stage_all(self.root()).await
    }

    /// 取消暂存指定路径。
    pub async fn unstage(&self, paths: &[String]) -> Result<(), GitError> {
        ops::unstage(self.root(), paths).await
    }

    /// 丢弃已跟踪文件的工作区改动。
    pub async fn discard(&self, paths: &[String]) -> Result<(), GitError> {
        ops::discard(self.root(), paths).await
    }

    /// 删除未跟踪文件/目录（不可恢复）。
    pub async fn discard_untracked(&self, paths: &[String]) -> Result<(), GitError> {
        ops::discard_untracked(self.root(), paths).await
    }

    /// 以当前索引创建提交。
    pub async fn commit(&self, message: &str, amend: bool) -> Result<(), GitError> {
        ops::commit(self.root(), message, amend).await
    }

    /// 针对指定的历史提交创建 fixup 提交。
    pub async fn commit_fixup(&self, oid: &str) -> Result<(), GitError> {
        ops::commit_fixup(self.root(), oid).await
    }

    /// 切换分支。
    pub async fn checkout(&self, branch: &str) -> Result<(), GitError> {
        ops::checkout(self.root(), branch).await
    }

    /// 游离头指针检出到指定提交。
    pub async fn checkout_detached(&self, oid: &str) -> Result<(), GitError> {
        ops::checkout_detached(self.root(), oid).await
    }

    /// 新建分支，可选指定起点与是否立即切换。
    pub async fn create_branch(
        &self,
        name: &str,
        start: Option<&str>,
        switch_to: bool,
    ) -> Result<(), GitError> {
        ops::create_branch(self.root(), name, start, switch_to).await
    }

    /// 重命名分支：保留上游配置与引用日志，比「新建 + 删除」少剪断历史关系。
    pub async fn rename_branch(&self, old: &str, new: &str) -> Result<(), GitError> {
        ops::rename_branch(self.root(), old, new).await
    }

    /// 删除分支；`force` 对应存在未合并提交的情况。
    pub async fn delete_branch(&self, name: &str, force: bool) -> Result<(), GitError> {
        ops::delete_branch(self.root(), name, force).await
    }

    /// 创建标签；`message` 非空时建附注标签。
    pub async fn create_tag(
        &self,
        name: &str,
        target: &str,
        message: Option<&str>,
    ) -> Result<(), GitError> {
        ops::create_tag(self.root(), name, target, message).await
    }

    /// 删除本地标签。
    pub async fn delete_tag(&self, name: &str) -> Result<(), GitError> {
        ops::delete_tag(self.root(), name).await
    }

    /// 拉取远端引用（不改工作区）。
    pub async fn fetch(&self) -> Result<(), GitError> {
        remote::fetch(self.root()).await
    }

    /// 推送分支。
    pub async fn push(&self, branch: Option<&str>, tags: bool) -> Result<(), GitError> {
        remote::push(self.root(), branch, tags).await
    }

    /// 把「到此提交为止」的历史推到该分支的上游远端（不改本地状态）。
    pub async fn push_up_to(&self, oid: &str, branch: &str) -> Result<(), GitError> {
        remote::push_up_to(self.root(), oid, branch).await
    }

    /// 拉取并整合上游。
    pub async fn pull(&self, rebase: bool) -> Result<(), GitError> {
        remote::pull(self.root(), rebase).await
    }

    /// 远端名列表。
    pub async fn remotes(&self) -> Result<Vec<String>, GitError> {
        remote::remotes(self.root()).await
    }

    /// 搁置项列表。
    pub async fn stashes(&self) -> Result<Vec<transom_core::StashEntry>, GitError> {
        stash::list(self.root()).await
    }

    /// 一条提交的 mbox 补丁文本（`git format-patch --stdout -1`）。
    pub async fn commit_patch(&self, oid: &str) -> Result<Vec<u8>, GitError> {
        patch::commit_patch(self.root(), oid).await
    }

    /// 该提交在远端托管服务上的网页地址；无远端或形态不认识时 `None`。
    pub async fn commit_url(&self, oid: &str) -> Result<Option<String>, GitError> {
        web::commit_url(self.root(), oid).await
    }

    /// 某个搁置项包含的文件清单（含未跟踪侧标记）。
    pub async fn stash_files(
        &self,
        selector: &str,
    ) -> Result<Vec<transom_core::StashFile>, GitError> {
        stash::files(self.root(), selector).await
    }

    /// 新建搁置项。
    pub async fn stash_save(&self, message: &str, include_untracked: bool) -> Result<(), GitError> {
        stash::save(self.root(), message, include_untracked).await
    }

    /// 应用搁置项（保留栈项）。
    pub async fn stash_apply(&self, selector: &str) -> Result<(), GitError> {
        stash::apply(self.root(), selector).await
    }

    /// 应用并弹出搁置项。
    pub async fn stash_pop(&self, selector: &str) -> Result<(), GitError> {
        stash::pop(self.root(), selector).await
    }

    /// 丢弃搁置项。
    pub async fn stash_drop(&self, selector: &str) -> Result<(), GitError> {
        stash::drop(self.root(), selector).await
    }

    /// 把分支头提交压进父提交；前置条件不满足时拒绝执行。
    pub async fn fold_tip_into_parent(
        &self,
        mode: transom_core::GitFoldMode,
    ) -> Result<(), GitError> {
        history::fold_tip_into_parent(self.root(), mode).await
    }

    /// 新建链接工作树。
    pub async fn add_worktree(
        &self,
        path: &str,
        branch: &str,
        start: Option<&str>,
    ) -> Result<(), GitError> {
        remote::add_worktree(self.root(), path, branch, start).await
    }

    /// 列出全部工作树。
    pub async fn worktrees(&self) -> Result<Vec<transom_core::WorktreeEntry>, GitError> {
        worktree::list(self.root()).await
    }

    /// 删除工作树。
    pub async fn remove_worktree(&self, path: &str, force: bool) -> Result<(), GitError> {
        worktree::remove(self.root(), path, force).await
    }

    /// 锁定工作树（`reason` 为空时不带原因）：锁定后 git 拒绝删除与 prune。
    pub async fn lock_worktree(&self, path: &str, reason: Option<&str>) -> Result<(), GitError> {
        worktree::lock(self.root(), path, reason).await
    }

    /// 解锁工作树。
    pub async fn unlock_worktree(&self, path: &str) -> Result<(), GitError> {
        worktree::unlock(self.root(), path).await
    }

    /// 重置当前分支到指定修订。
    pub async fn reset(&self, target: &str, kind: ResetKind) -> Result<(), GitError> {
        history::reset(self.root(), target, kind).await
    }

    /// 撤销指定提交（生成反向提交）。
    pub async fn revert(&self, oid: &str) -> Result<(), GitError> {
        history::revert(self.root(), oid).await
    }

    /// 拣选指定提交。
    pub async fn cherry_pick(&self, oid: &str) -> Result<(), GitError> {
        history::cherry_pick(self.root(), oid).await
    }

    /// 合并指定引用到当前分支。
    pub async fn merge(&self, reference: &str) -> Result<(), GitError> {
        history::merge(self.root(), reference).await
    }

    /// 变基到指定上游。
    pub async fn rebase_onto(&self, upstream: &str) -> Result<(), GitError> {
        history::rebase_onto(self.root(), upstream).await
    }

    /// 取 `<base>..HEAD` 的变基待办清单（旧→新，默认全 pick）。
    pub async fn rebase_todo(
        &self,
        base: &str,
    ) -> Result<Vec<transom_core::GitRebaseStep>, GitError> {
        rebase::todo(self.root(), base).await
    }

    /// 按用户编辑后的清单执行交互式变基。
    pub async fn rebase_with_todo(
        &self,
        base: &str,
        steps: &[transom_core::GitRebaseStep],
    ) -> Result<(), GitError> {
        rebase::run_todo(self.root(), base, steps).await
    }

    /// 中止进行中的变基。
    pub async fn rebase_abort(&self) -> Result<(), GitError> {
        history::rebase_abort(self.root()).await
    }
}
