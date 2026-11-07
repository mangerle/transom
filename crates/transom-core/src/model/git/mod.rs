//! Git 领域模型：项目记录与仓库快照契约。
//!
//! 设计原理（Why）：
//! - 快照类型放在 core 而不是 `transom-git`，与 [`crate::SystemStats`] 同一口径：
//!   命令/事件总线、控制器与界面只认 core 的契约，git 引擎 crate 负责把
//!   子进程输出折算成这些类型。将来若替换实现（例如改用常驻 git 进程或
//!   libgit2），界面层不需要跟着改；
//! - [`GitProject`] 是唯一需要落盘的记录，因此只有它带 serde；快照类型
//!   只在内存里流动，不假装可持久化；
//! - 状态用枚举表达而不是布尔位堆砌：一个文件同时具有「索引侧」与
//!   「工作区侧」两个独立状态，[`StatusRecord::Tracked`] 携带两位状态码
//!   既保留了 `AM`（已新增又被改）这类组合，又让冲突/未跟踪/忽略互斥归位。

pub mod diff;
pub mod log;
pub mod ops;
pub mod project;
pub mod status;

pub use diff::{DiffContent, GitFoldMode, GitSide};
pub use log::{
    BranchEntry, CommitEntry, CommitFile, LogPage, LogQuery, LogScope, RefDecor, RefKind, RepoInfo,
    TagEntry,
};
pub use ops::{
    GitAction, GitCheckout, GitPushOptions, GitRebaseAction, GitRebaseStep, GitResetKind,
    PushTagsMode, StashEntry, StashFile, WorktreeEntry,
};
pub use project::{GitFolder, GitProject, GitRegistry};
pub use status::{ChangeCode, HeadState, RepoStatus, StatusEntry, StatusRecord};
