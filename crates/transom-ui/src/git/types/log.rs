//! Git 日志状态与跳转定位模型。

use gpui_kit::component::VirtualListScrollHandle;
use transom_core::{CommitEntry, GitRequestId};
use transom_git::GraphRow;

/// Git 日志面板状态。
pub(crate) struct GitLogState {
    pub entries: Vec<CommitEntry>,
    /// 与 `entries` 一一对应的图行（列号与连线）。
    pub rows: Vec<GraphRow>,
    /// 泳道分配的跨页状态，翻页时续接列号。
    pub graph: transom_git::GraphState,
    pub has_more: bool,
    pub loading: bool,
    pub error: Option<String>,
    /// 提交表的虚拟滚动句柄。
    pub scroll_handle: VirtualListScrollHandle,
    /// 查询范围（当前分支 / 全部引用）。
    pub scope: transom_core::LogScope,
    /// 当前在途的日志请求：`(请求 id, 是否为追加页)`。
    ///
    /// 为什么记这个：分页是"追加"，重新加载是"替换"，两者对同一份 `entries`
    /// 的动作不同。迟到的旧请求必须按 id 比对后丢弃，否则下拉加载的结果会
    /// 把用户刚切换的过滤条件覆盖掉。
    pub request: Option<(GitRequestId, bool)>,
    /// 表头列显示开关。
    pub columns: LogColumns,
    /// 待落地的跳转目标（IDEA 的「转到父提交 / 转到子提交」）。
    ///
    /// 设计原理（Why）：目标行可能还没加载进来——父提交在下一页、子提交在更早的
    /// 历史里。这里记住「找谁」而不是「第几行」，日志页每次落地后继续找，
    /// 找不到就顺着分页往下翻，翻到头才放弃并如实提示。
    pub jump: Option<LogJump>,
    /// 为落地这次跳转已经追加过几页。
    ///
    /// 设计原理（Why）：大仓库的历史可能有几十万条，找不到目标就无限翻页等于
    /// 把子进程队列打满。超过 [`MAX_JUMP_PAGES`] 页即停手并提示。
    pub jump_pages: usize,
}

/// 尚未落地的提交跳转目标。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LogJump {
    /// 跳到该提交的第一个父提交。
    Parent(String),
    /// 跳到第一个把该提交当作父提交的子提交。
    Child(String),
}

/// 一次跳转最多追加的页数：再远就提示找不到，而不是一直翻页。
pub(crate) const MAX_JUMP_PAGES: usize = 5;

impl LogJump {
    /// 在当前已加载的页里找目标行。
    pub(crate) fn resolve(&self, entries: &[transom_core::CommitEntry]) -> Option<usize> {
        match self {
            Self::Parent(oid) => entries
                .iter()
                .find(|entry| entry.oid == *oid)
                .and_then(|entry| entry.parents.first())
                .and_then(|parent| entries.iter().position(|entry| entry.oid == *parent)),
            Self::Child(oid) => entries
                .iter()
                .position(|entry| entry.parents.iter().any(|parent| parent == oid)),
        }
    }
}

/// 日志表的列显示开关（信息列恒定）。
///
/// 设计原理（Why）：对齐 IDEA 日志表头的右键勾选。窄窗口下关掉作者或日期
/// 能把信息列拉宽，因此它是纯视图参数，不影响取数与分页。
#[derive(Debug, Clone, Copy)]
pub(crate) struct LogColumns {
    pub author: bool,
    pub time: bool,
}

impl Default for LogColumns {
    /// 默认两列都显示：与 IDEA 的新建布局一致，隐藏列必须是用户主动选择。
    fn default() -> Self {
        Self {
            author: true,
            time: true,
        }
    }
}

/// 表头菜单里可勾选的一列。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LogColumn {
    Author,
    Time,
}

impl LogColumn {
    /// 该列在 [`GitLogState::columns`] 里的当前开关。
    pub(crate) fn is_shown(self, columns: &LogColumns) -> bool {
        match self {
            Self::Author => columns.author,
            Self::Time => columns.time,
        }
    }

    /// 翻转该列的开关。
    pub(crate) fn flip(self, columns: &mut LogColumns) {
        match self {
            Self::Author => columns.author = !columns.author,
            Self::Time => columns.time = !columns.time,
        }
    }

    /// 列名文案键。
    pub(crate) fn i18n_key(self) -> &'static str {
        match self {
            Self::Author => "git.log.column.author",
            Self::Time => "git.log.column.time",
        }
    }
}

impl Default for GitLogState {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            rows: Vec::new(),
            graph: transom_git::GraphState::default(),
            has_more: false,
            loading: false,
            error: None,
            scroll_handle: VirtualListScrollHandle::new(),
            scope: transom_core::LogScope::CurrentBranch,
            request: None,
            columns: LogColumns::default(),
            jump: None,
            jump_pages: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use transom_core::CommitEntry;

    /// 造一条只关心 oid 与父提交的最小记录。
    fn commit(oid: &str, parents: &[&str]) -> CommitEntry {
        CommitEntry {
            oid: oid.to_string(),
            parents: parents.iter().map(|parent| parent.to_string()).collect(),
            author_name: String::new(),
            author_email: String::new(),
            author_time: 0,
            subject: String::new(),
            body: String::new(),
            refs: Vec::new(),
        }
    }

    /// 日志页是新→旧排序，父提交通常还在下一页：没加载到就返回 None 交给翻页。
    #[test]
    fn jump_parent_resolves_only_when_loaded() {
        let page = vec![commit("b", &["a"]), commit("a", &[]), commit("x", &["y"])];
        assert_eq!(LogJump::Parent("b".into()).resolve(&page), Some(1));
        // 父提交还没加载（在下一页）时返回 None，由调用方继续翻页。
        assert_eq!(LogJump::Parent("x".into()).resolve(&page), None);
        // 目标提交本身不在本页，同样交回 None。
        assert_eq!(LogJump::Parent("missing".into()).resolve(&page), None);
    }

    /// 子提交靠「谁的父里有我」定位，与父提交跳转共用一套解析。
    #[test]
    fn jump_child_finds_descendant_row() {
        let page = vec![commit("c", &["b"]), commit("b", &["a"])];
        assert_eq!(LogJump::Child("b".into()).resolve(&page), Some(0));
        // 头提交没有子提交。
        assert_eq!(LogJump::Child("c".into()).resolve(&page), None);
    }

    /// 合并提交的跳转按第一父，与 IDEA 的「第一个父提交」口径一致。
    #[test]
    fn jump_parent_uses_first_parent_of_merge() {
        let page = vec![
            commit("m", &["p1", "p2"]),
            commit("p2", &[]),
            commit("p1", &[]),
        ];
        assert_eq!(LogJump::Parent("m".into()).resolve(&page), Some(2));
    }

    /// 列开关默认全开，翻转互不影响。
    #[test]
    fn log_columns_toggle_independently() {
        let mut columns = LogColumns::default();
        assert!(columns.author && columns.time);
        LogColumn::Author.flip(&mut columns);
        assert!(!columns.author && columns.time);
        assert!(!LogColumn::Author.is_shown(&columns));
        assert!(LogColumn::Time.is_shown(&columns));
    }
}
