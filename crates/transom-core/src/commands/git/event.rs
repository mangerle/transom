//! Git 领域事件与搁置操作标识。

use std::path::PathBuf;

use crate::error::AppError;
use crate::ids::{GitProjectId, GitRequestId};
use crate::model::git::{
    BranchEntry, CommitEntry, CommitFile, ConflictSides, DiffContent, GitAction, GitRebaseStep,
    LogPage, RepoInfo, RepoStatus, ShelfEntry, StashEntry, StashFile, TagEntry, WorktreeEntry,
};

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
