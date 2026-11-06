//! 领域契约的再导出。
//!
//! 设计原理（Why）：仓库快照类型归 `transom-core`（与 `SystemStats` 同一口径，
//! 见 `model/git.rs` 的说明），本 crate 只负责把 git 输出折算成它们。
//! 这里做模块内再导出，是为了让解析代码继续以 `crate::types::X` 引用，
//! 而不必在每个模块顶部重复一长串 `transom_core::` 名字。

pub use transom_core::{
    BranchEntry, ChangeCode, CommitEntry, CommitFile, DiffContent, HeadState, LogPage, LogQuery,
    LogScope, RefDecor, RefKind, RepoInfo, RepoStatus, StatusEntry, StatusRecord, TagEntry,
};
