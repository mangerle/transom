//! Git 领域命令与事件。
//!
//! 将 Git 面板与后台 git 引擎之间的命令与事件收敛为独立子系统契约。

use std::path::PathBuf;

use crate::error::AppError;
use crate::ids::{GitProjectId, GitRequestId};
use crate::model::git::{
    BranchEntry, CommitFile, DiffContent, GitAction, GitCheckout, GitFoldMode, GitProject,
    GitRebaseStep, GitResetKind, GitSide, LogPage, LogQuery, RepoInfo, RepoStatus, StashEntry,
    StashFile, TagEntry, WorktreeEntry,
};

/// Git 领域命令：全部发往控制器派发的后台任务。
pub enum GitCommand {
    /// 刷新仓库状态（porcelain status + 领先/落后 + HEAD）。
    ///
    /// `git status` 默认不列忽略项。引擎仍保留读取忽略清单的能力，只是
    /// 界面不再另外造一套 git 里没有的视图。
    Refresh {
        project: GitProject,
    },
    /// 分页查询提交历史。
    LoadLog {
        project: GitProject,
        request_id: GitRequestId,
        query: LogQuery,
    },
    /// 单个提交改动的文件清单。
    LoadCommitFiles {
        project: GitProject,
        request_id: GitRequestId,
        oid: String,
    },
    /// 分支与标签列表。
    LoadRefs {
        project: GitProject,
    },
    /// 读取某文件两侧的文本内容。
    LoadDiff {
        project: GitProject,
        request_id: GitRequestId,
        path: String,
        old: GitSide,
        new: GitSide,
    },
    Stage {
        project: GitProject,
        paths: Vec<String>,
    },
    StageAll {
        project: GitProject,
    },
    Unstage {
        project: GitProject,
        paths: Vec<String>,
    },
    /// 丢弃已跟踪文件的工作区改动。
    Discard {
        project: GitProject,
        paths: Vec<String>,
    },
    /// 删除未跟踪文件（不可恢复，界面必须已二次确认）。
    DeleteUntracked {
        project: GitProject,
        paths: Vec<String>,
    },
    Commit {
        project: GitProject,
        message: String,
        amend: bool,
    },
    Checkout {
        project: GitProject,
        target: GitCheckout,
    },
    CreateBranch {
        project: GitProject,
        name: String,
        start: Option<String>,
        switch_to: bool,
    },
    /// 重命名分支：`git branch -m`，保留上游配置与引用日志。
    RenameBranch {
        project: GitProject,
        old: String,
        new: String,
    },
    DeleteBranch {
        project: GitProject,
        name: String,
        force: bool,
    },
    CreateTag {
        project: GitProject,
        name: String,
        target: String,
        message: Option<String>,
    },
    DeleteTag {
        project: GitProject,
        name: String,
    },
    /// `git worktree add`：从指定起点建新工作树并建分支。
    AddWorktree {
        project: GitProject,
        path: String,
        branch: String,
        start: Option<String>,
    },
    Fetch {
        project: GitProject,
    },
    Push {
        project: GitProject,
        /// 目标分支；`None` 表示当前分支。
        branch: Option<String>,
        tags: bool,
    },
    Pull {
        project: GitProject,
        rebase: bool,
    },
    /// 把「到此提交为止」的历史推到分支的上游远端（IDEA 的「推送此前所有提交」）。
    PushUpTo {
        project: GitProject,
        oid: String,
        branch: String,
    },
    /// 添加项目时对目录做一次仓库探测（含子进程，不能卡在 UI 线程）。
    Probe {
        request_id: GitRequestId,
        path: PathBuf,
    },
    /// 对一个已有目录执行 `git init`。
    Init {
        request_id: GitRequestId,
        path: PathBuf,
    },
    /// 将当前分支重置到指定修订。
    ResetTo {
        project: GitProject,
        target: String,
        kind: GitResetKind,
    },
    /// 撤销指定提交（生成反向提交）。
    Revert {
        project: GitProject,
        oid: String,
    },
    /// 拣选指定提交。
    CherryPick {
        project: GitProject,
        oid: String,
    },
    /// 合并指定引用到当前分支。
    Merge {
        project: GitProject,
        reference: String,
    },
    /// 将当前分支变基到指定上游。
    RebaseOnto {
        project: GitProject,
        upstream: String,
    },
    /// 中止进行中的变基。
    RebaseAbort {
        project: GitProject,
    },
    /// 列出工作树。
    ListWorktrees {
        project: GitProject,
        request_id: GitRequestId,
    },
    /// 删除工作树。
    RemoveWorktree {
        project: GitProject,
        path: String,
        force: bool,
    },
    /// 取 `<base>..HEAD` 的交互式变基待办清单（旧→新）。
    RebaseTodo {
        project: GitProject,
        request_id: GitRequestId,
        base: String,
    },
    /// 按用户编辑后的清单执行交互式变基。
    ///
    /// 设计原理（Why）：清单由界面回传而不是引擎重算——用户可能已经改过动作
    /// 与顺序，重算会把编辑丢掉；提交号在引擎侧再校验一次十六进制，
    /// 不让界面文本变成 git 参数。
    RebaseRun {
        project: GitProject,
        base: String,
        steps: Vec<GitRebaseStep>,
    },
    /// 把分支头提交压进它的父提交（IDEA 的「压缩到／优选到」单提交形态）。
    FoldTipIntoParent {
        project: GitProject,
        mode: GitFoldMode,
    },
    /// 锁定工作树：IDEA 的「锁定」用于让临时工作树不被 prune 掉。
    LockWorktree {
        project: GitProject,
        path: String,
        reason: Option<String>,
    },
    /// 解锁工作树。
    UnlockWorktree {
        project: GitProject,
        path: String,
    },
    /// 列出搁置项。
    ListStashes {
        project: GitProject,
        request_id: GitRequestId,
    },
    /// 列出某个搁置项包含的文件。
    ///
    /// 为什么不跟列表一起返回：一个仓库可能有几十条搁置，每条都要一次
    /// `stash show` 加一次 `ls-tree`；IDEA 也是展开时才取清单，
    /// 折叠的行不值得为它起子进程。
    ListStashFiles {
        project: GitProject,
        request_id: GitRequestId,
        selector: String,
    },
    /// 新建搁置项。
    StashSave {
        project: GitProject,
        message: String,
        include_untracked: bool,
    },
    /// 应用搁置项（保留栈项）。
    StashApply {
        project: GitProject,
        selector: String,
    },
    /// 应用并弹出搁置项。
    StashPop {
        project: GitProject,
        selector: String,
    },
    /// 丢弃搁置项（不可恢复）。
    StashDrop {
        project: GitProject,
        selector: String,
    },
    /// 取一条提交在远端托管服务上的网页地址。
    ///
    /// 设计原理（Why）：地址要读 `git remote get-url`（子进程），不能在渲染线程里
    /// 现取，所以走命令总线；结果可能为空（无远端或形态不认识），由界面提示，
    /// 而不是打开一个拼出来的假地址。
    ResolveCommitUrl {
        project: GitProject,
        request_id: GitRequestId,
        oid: String,
    },
    /// 把一条提交打成补丁并写到用户选定的路径。
    ///
    /// 设计原理（Why）：落盘路径来自界面里的系统保存对话框，是用户明示意图，
    /// 因此命令直接带路径，控制器不必把十几 MB 的补丁文本经事件总线回传；
    /// 失败按普通写动作经 `ActionDone` 回报。
    CreatePatch {
        project: GitProject,
        oid: String,
        path: PathBuf,
    },
    /// 从远端克隆仓库到本机目录。
    ///
    /// 设计原理（Why）：克隆不属于任何「已登记项目」——仓库还不存在，所以这条命令
    /// 不带 `project`，只带请求号、地址与目标目录；进度由
    /// [`GitAppEvent::CloneProgress`] 多次回报，结束由
    /// [`GitAppEvent::CloneDone`] 收口，界面按请求号落到对应的克隆标签页。
    Clone {
        request_id: GitRequestId,
        url: String,
        dest: PathBuf,
        /// 浅克隆深度；`None` 表示拉完整历史。
        depth: Option<u32>,
    },
    /// 中止在途读请求（视图关闭时）。
    Cancel {
        request_id: GitRequestId,
    },
}

/// Git 面板事件：全部是「一次读命令的结果」或「一次写动作的收尾」。
///
/// 设计原理（Why）：快照类型按框（`Box`）传递，日志一页可达数百条提交，
/// 直接内嵌会让整个事件枚举变大一个数量级，而事件在通道里只是搬运一次。
#[derive(Debug, Clone)]
pub enum GitAppEvent {
    StatusLoaded {
        project_id: GitProjectId,
        status: Box<RepoStatus>,
    },
    LogLoaded {
        project_id: GitProjectId,
        request_id: GitRequestId,
        page: Box<LogPage>,
    },
    CommitFilesLoaded {
        project_id: GitProjectId,
        request_id: GitRequestId,
        oid: String,
        files: Vec<CommitFile>,
    },
    RefsLoaded {
        project_id: GitProjectId,
        branches: Vec<BranchEntry>,
        tags: Vec<TagEntry>,
        /// 跟踪文件清单：日志的路径过滤下拉直接用，省一次独立请求。
        paths: Vec<String>,
    },
    DiffLoaded {
        project_id: GitProjectId,
        request_id: GitRequestId,
        diff: Box<DiffContent>,
    },
    /// 克隆输出：一次事件可能带多行（控制器按时间窗合并后再发）。
    ///
    /// 为什么不逐行发：事件通道是有界的且用 `try_send`，慢网络上 `git clone`
    /// 每秒能刷几十行，逐行发会把队列打满并静默丢进度——用户看到的反而是断档。
    CloneProgress {
        request_id: GitRequestId,
        text: String,
    },
    /// 克隆结束：`error` 为空表示成功，界面按 `dest` 把仓库登记进项目列表。
    CloneDone {
        request_id: GitRequestId,
        dest: PathBuf,
        error: Option<AppError>,
    },
    /// 读请求失败：界面按 `request_id` 结束对应加载态并显示错误。
    ///
    /// 为什么不复用 [`Self::ActionDone`]：读请求没有「动作」语义，且必须带
    /// `request_id` 才能把错误落到正确的视图，混用会让界面分不清该关哪个 spinner。
    ReadFailed {
        project_id: GitProjectId,
        request_id: GitRequestId,
        error: AppError,
    },
    /// 写动作完成：`error` 为空表示成功，界面随后自行刷新。
    ActionDone {
        project_id: GitProjectId,
        action: GitAction,
        error: Option<AppError>,
    },
    /// 工作树列表。
    WorktreesLoaded {
        project_id: GitProjectId,
        request_id: GitRequestId,
        entries: Vec<WorktreeEntry>,
    },
    /// 搁置项列表。
    StashesLoaded {
        project_id: GitProjectId,
        request_id: GitRequestId,
        entries: Vec<StashEntry>,
    },
    /// 提交的网页地址；`url` 为空表示无法确定（无远端或托管形态不认识）。
    CommitUrlLoaded {
        project_id: GitProjectId,
        request_id: GitRequestId,
        oid: String,
        url: Option<String>,
    },
    /// 交互式变基的待办清单（旧→新）。
    RebaseTodoLoaded {
        project_id: GitProjectId,
        request_id: GitRequestId,
        base: String,
        steps: Vec<GitRebaseStep>,
    },
    /// 某个搁置项的文件清单。
    StashFilesLoaded {
        project_id: GitProjectId,
        request_id: GitRequestId,
        selector: String,
        files: Vec<StashFile>,
    },
    /// 目录探测结果：`info` 为空表示该目录不是仓库（界面走「初始化」分支）。
    Probed {
        request_id: GitRequestId,
        path: PathBuf,
        info: Option<Box<RepoInfo>>,
        error: Option<AppError>,
    },
    /// `git init` 结果。
    Inited {
        request_id: GitRequestId,
        path: PathBuf,
        error: Option<AppError>,
    },
}
