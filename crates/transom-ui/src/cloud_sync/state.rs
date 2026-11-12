//! 云同步运行期状态：当前动作、检查结果、冲突与有界历史。

use super::history::{SyncHistoryEntry, SyncHistoryStatus};
use super::scheduler::CloudSyncScheduler;
use std::collections::HashSet;
use transom_core::{SyncCheckReport, SyncOperation, SyncTrigger};

/// 历史筛选类别。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum SyncHistoryFilter {
    #[default]
    All,
    Success,
    Failed,
    Conflict,
}

impl SyncHistoryFilter {
    /// 当前筛选是否接受该历史项。
    pub(crate) fn matches(self, entry: &SyncHistoryEntry) -> bool {
        match self {
            Self::All => true,
            Self::Success => entry.status == SyncHistoryStatus::Success,
            Self::Failed => entry.status == SyncHistoryStatus::Failed,
            Self::Conflict => entry.status == SyncHistoryStatus::Conflict,
        }
    }
}

/// 侧边栏与设置页共享的同步状态快照。
pub(crate) struct CloudSyncViewState {
    pub(crate) status: SyncHistoryStatus,
    pub(crate) active_operation: Option<SyncOperation>,
    pub(crate) active_trigger: Option<SyncTrigger>,
    pub(crate) message: Option<String>,
    pub(crate) last_check: Option<SyncCheckReport>,
    pub(crate) conflict: Option<SyncCheckReport>,
    pub(crate) history: Vec<SyncHistoryEntry>,
    pub(crate) history_filter: SyncHistoryFilter,
    pub(crate) expanded_history: HashSet<String>,
    pub(crate) history_visible_limit: usize,
    pub(crate) scheduler: CloudSyncScheduler,
}

impl CloudSyncViewState {
    /// 初始历史可见条数。
    const INITIAL_HISTORY_LIMIT: usize = 20;

    /// 创建空状态；历史稍后由启动任务异步装载。
    pub(crate) fn new() -> Self {
        Self {
            status: SyncHistoryStatus::Idle,
            active_operation: None,
            active_trigger: None,
            message: None,
            last_check: None,
            conflict: None,
            history: Vec::with_capacity(64),
            history_filter: SyncHistoryFilter::All,
            expanded_history: HashSet::with_capacity(8),
            history_visible_limit: Self::INITIAL_HISTORY_LIMIT,
            scheduler: CloudSyncScheduler::default(),
        }
    }

    /// 开始一次动作；在途状态同时驱动按钮禁用与旋转指示。
    pub(crate) fn begin(
        &mut self,
        operation: SyncOperation,
        trigger: SyncTrigger,
        message: impl Into<String>,
    ) {
        self.status = SyncHistoryStatus::Running;
        self.active_operation = Some(operation);
        self.active_trigger = Some(trigger);
        self.message = Some(message.into());
    }

    /// 动作结束时清理在途状态并写入历史。
    pub(crate) fn finish(&mut self, entry: SyncHistoryEntry) {
        self.status = entry.status;
        self.active_operation = None;
        self.active_trigger = None;
        self.message = Some(entry.message.clone());
        if entry.status == SyncHistoryStatus::Conflict {
            // 冲突详情由检查结果单独维护，历史只保留摘要。
        } else {
            self.conflict = None;
        }
        self.history.insert(0, entry);
        self.history.truncate(super::history::MAX_HISTORY_ENTRIES);
        self.history_visible_limit = self.history_visible_limit.max(Self::INITIAL_HISTORY_LIMIT);
    }

    /// 装载已持久化历史，按时间倒序保留固定上限。
    pub(crate) fn replace_history(&mut self, mut history: Vec<SyncHistoryEntry>) {
        history.sort_by_key(|entry| std::cmp::Reverse(entry.at_ms));
        history.truncate(super::history::MAX_HISTORY_ENTRIES);
        self.history = history;
    }

    /// 应用一次远端检查结果；分叉与首次基线不确定都进入冲突预览。
    pub(crate) fn apply_check(&mut self, report: SyncCheckReport) {
        let needs_attention = matches!(
            report.relation,
            transom_core::SyncRelation::Diverged | transom_core::SyncRelation::RemoteMissing
        );
        self.last_check = Some(report.clone());
        if needs_attention {
            self.conflict = Some(report);
        } else {
            self.conflict = None;
        }
    }

    /// 当前是否正在执行同步动作。
    pub(crate) fn is_running(&self) -> bool {
        self.active_operation.is_some()
    }

    /// 切换历史项展开状态。
    pub(crate) fn toggle_history(&mut self, id: &str) {
        if !self.expanded_history.insert(id.to_string()) {
            self.expanded_history.remove(id);
        }
    }

    /// 增加一批历史可见条数。
    pub(crate) fn show_more_history(&mut self) {
        self.history_visible_limit = self.history_visible_limit.saturating_add(20);
    }
}

impl Default for CloudSyncViewState {
    fn default() -> Self {
        Self::new()
    }
}
