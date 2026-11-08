//! Git 领域命令与事件。
//!
//! 将 Git 面板与后台 git 引擎之间的命令与事件收敛为独立子系统契约。

use std::path::PathBuf;

use crate::error::AppError;
use crate::ids::{GitProjectId, GitRequestId};
use crate::model::git::{
    BranchEntry, CommitEntry, CommitFile, ConflictSides, DiffContent, GitAction, GitCheckout,
    GitFoldMode, GitProject, GitPushOptions, GitRebaseStep, GitResetKind, GitSide, LogPage,
    LogQuery, RepoInfo, RepoStatus, ShelfEntry, StashEntry, StashFile, TagEntry, WorktreeEntry,
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
        fixup_target: Option<String>,
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
        /// 推送详细选项。
        options: GitPushOptions,
    },
    /// 查询待推送到目标分支的提交列表（Outgoing Commits）。
    LoadOutgoingCommits {
        project: GitProject,
        request_id: GitRequestId,
        branch: String,
        remote: Option<String>,
        remote_branch: Option<String>,
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
    /// 读取项目的离线搁置项（Shelf）列表。
    ///
    /// 设计原理（Why）：搁置项存放在应用数据目录的 JSON 里而不是 git 仓库内，
    /// 读取是阻塞文件 IO。走命令总线是为了让界面不直接依赖 `transom-store`，
    /// 由控制器把读盘调度到阻塞线程池；`request_id` 用于丢弃关闭标签后迟到的结果。
    LoadShelves {
        project: GitProject,
        request_id: GitRequestId,
    },
    /// 把勾选的改动导出为离线搁置项，并从工作区抽离这些改动。
    ///
    /// `name` 与 `branch` 由界面传入：名称含本地化文案，分支取自界面当前
    /// 状态快照；控制器只负责补丁导出、工作区丢弃与落盘，不重新推断这两个字段。
    Shelve {
        project: GitProject,
        request_id: GitRequestId,
        name: String,
        branch: String,
        files: Vec<String>,
    },
    /// 应用一条离线搁置项的补丁；`pop` 为真时应用成功后删除该条目。
    ///
    /// 补丁文本随命令携带而不是让控制器按 id 回读：界面手里已有补丁，
    /// 多一次读盘只会让「应用」与「条目被并发删除」之间的竞态更难解释。
    ApplyShelf {
        project: GitProject,
        request_id: GitRequestId,
        shelf_id: String,
        patch: String,
        pop: bool,
    },
    /// 删除一条离线搁置项（不可恢复）。
    DropShelf {
        project: GitProject,
        request_id: GitRequestId,
        shelf_id: String,
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
    /// 解决冲突：采用本地/当前版本。
    ResolveConflictOurs {
        project: GitProject,
        paths: Vec<String>,
    },
    /// 解决冲突：采用传入版本。
    ResolveConflictTheirs {
        project: GitProject,
        paths: Vec<String>,
    },
    /// 标记冲突文件已手动解决。
    ResolveConflictMark {
        project: GitProject,
        paths: Vec<String>,
    },
    /// 读取冲突文件的三方内容视图。
    LoadConflictSides {
        project: GitProject,
        request_id: GitRequestId,
        path: String,
    },
    /// 中止在途读请求（视图关闭时）。
    Cancel {
        request_id: GitRequestId,
    },
}

/// 离线搁置项（Shelf）操作类别。
///
/// 设计原理（Why）：搁置项的失败可能发生在 git 补丁（导出、应用、丢弃工作区）
/// 或本地存储（读列表、落盘）任一步，界面需要按步骤区分日志级别与提示口径；
/// 用一个独立小枚举表达，而不是把操作名塞进错误文案里做字符串判断。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitShelfOperation {
    /// 读取本地搁置项列表。
    Load,
    /// 创建搁置项（导出补丁并从工作区抽离改动）。
    Shelve,
    /// 应用或弹回搁置项。
    Apply,
    /// 删除搁置项。
    Drop,
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
    /// 待推送提交列表加载完成。
    OutgoingCommitsLoaded {
        project_id: GitProjectId,
        request_id: GitRequestId,
        branch: String,
        remote_branch: String,
        commits: Vec<CommitEntry>,
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
    /// 离线搁置项列表读取完成。
    ShelvesLoaded {
        project_id: GitProjectId,
        request_id: GitRequestId,
        shelves: Vec<ShelfEntry>,
    },
    /// 离线搁置项创建完成：补丁已导出并落盘。
    ///
    /// 工作区改动通常已被抽离；个别文件抽离失败时改动保留在原处，
    /// 控制器记录告警但不阻塞本次创建（补丁已可恢复，丢数据风险更高的是反向顺序）。
    Shelved {
        project_id: GitProjectId,
        request_id: GitRequestId,
        entry: ShelfEntry,
    },
    /// 离线搁置项应用完成：补丁已应用到工作区。
    ///
    /// `pop` 为真且条目删除失败时仍发本事件，失败放在 `cleanup_error`：
    /// 补丁应用成功是既成事实，若并入 [`Self::ShelfFailed`] 会让界面误报
    /// 「应用失败」，掩盖工作区已经被改写的事实。
    ShelfApplied {
        project_id: GitProjectId,
        request_id: GitRequestId,
        shelf_id: String,
        pop: bool,
        /// `pop` 为真时删除条目的结果；`Some` 表示补丁已应用但条目仍在存储中。
        cleanup_error: Option<AppError>,
    },
    /// 离线搁置项删除完成（条目本就不存在时同样按成功回报，删除是幂等的）。
    ShelfDropped {
        project_id: GitProjectId,
        request_id: GitRequestId,
        shelf_id: String,
    },
    /// 离线搁置项操作失败：界面按操作类别记录日志并提示，不再单独刷新。
    ///
    /// 不复用 [`Self::ReadFailed`] 或 [`Self::ActionDone`]：搁置操作同时涉及
    /// git 补丁与本地存储，失败时界面需要知道是哪一步（读列表、创建、应用、删除）
    /// 出了问题，才能给出对应的恢复提示。
    ShelfFailed {
        project_id: GitProjectId,
        request_id: GitRequestId,
        operation: GitShelfOperation,
        error: AppError,
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
    /// 冲突文件的三方内容加载完成。
    ConflictSidesLoaded {
        project_id: GitProjectId,
        request_id: GitRequestId,
        path: String,
        sides: Box<ConflictSides>,
    },
}
