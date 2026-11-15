//! 历史改写与整合动作意图定义。

/// 历史改写与整合动作（界面意图，落到具体命令）。
pub(crate) enum GitHistoryAction {
    Reset {
        target: String,
        kind: transom_core::GitResetKind,
    },
    Revert(String),
    CherryPick(String),
    Merge(String),
    Rebase(String),
    RebaseAbort,
}
