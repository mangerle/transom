//! 云同步视图实体：集中持有同步状态、历史写盘器与拉取后台任务。
//!
//! # 设计原理（Why）
//! - `CloudSyncViewState`、`SyncHistoryWriter` 与 `sync_pull` 后台任务原本散落在
//!   `AppState` 与 `TaskHub` 中，导致云同步动作与侧栏渲染必须持有 `&mut AppShell`；
//! - 收拢进独立 `Entity<CloudSyncView>` 后，状态变更与侧栏局部重绘由实体自治，
//!   仅通过 [`CloudSyncEvent`] 向外壳上报打开设置或执行同步动作等跨领域意图。

use super::history::{SyncHistoryEntry, SyncHistoryStatus, SyncHistoryWriter};
use super::state::{CloudSyncViewState, SyncHistoryFilter};
use crate::palette::Palette;
use gpui_kit::*;
use transom_core::{SyncCheckReport, SyncOperation, SyncTrigger};

/// 云同步侧栏操作意图。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CloudSyncSidebarAction {
    GitSyncNow,
    Check,
    Test,
    Push,
    Pull,
    ForcePull,
    ForcePush,
}

/// 云同步实体向外壳上报的意图事件。
#[derive(Debug, Clone)]
pub(crate) enum CloudSyncEvent {
    OpenSettings,
    TriggerAction(CloudSyncSidebarAction),
}

/// 一次历史记录的参数集合，避免多字段平铺造成调用点顺序错误。
pub(crate) struct SyncHistoryRecord<'a> {
    pub(crate) operation: SyncOperation,
    pub(crate) trigger: SyncTrigger,
    pub(crate) status: SyncHistoryStatus,
    pub(crate) provider: &'a str,
    pub(crate) duration_ms: u64,
    pub(crate) sessions: usize,
    pub(crate) folders: usize,
    pub(crate) bytes: u64,
    pub(crate) message: String,
}

/// 云同步独立实体。
pub(crate) struct CloudSyncView {
    pub(crate) state: CloudSyncViewState,
    pub(crate) history_writer: Option<SyncHistoryWriter>,
    pub(crate) pull_task: Option<Task<()>>,
}

impl EventEmitter<CloudSyncEvent> for CloudSyncView {}

impl Render for CloudSyncView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::from(cx);
        super::sidebar::render_cloud_sync_sidebar_view(self, window, cx, &p)
    }
}

impl CloudSyncView {
    /// 创建云同步实体并挂载后台历史写盘线程。
    pub(crate) fn new(history_writer: Option<SyncHistoryWriter>) -> Self {
        Self {
            state: CloudSyncViewState::new(),
            history_writer,
            pull_task: None,
        }
    }

    pub(crate) fn is_busy_for_auto_tick(&self) -> bool {
        self.state.is_running() || self.pull_task.is_some()
    }

    pub(crate) fn replace_history(
        &mut self,
        history: Vec<SyncHistoryEntry>,
        cx: &mut Context<Self>,
    ) {
        self.state.replace_history(history);
        cx.notify();
    }

    pub(crate) fn toggle_history(&mut self, id: &str, cx: &mut Context<Self>) {
        self.state.toggle_history(id);
        cx.notify();
    }

    pub(crate) fn show_more_history(&mut self, cx: &mut Context<Self>) {
        self.state.show_more_history();
        cx.notify();
    }

    pub(crate) fn set_history_filter(&mut self, filter: SyncHistoryFilter, cx: &mut Context<Self>) {
        self.state.history_filter = filter;
        self.state.history_visible_limit = 20;
        cx.notify();
    }

    pub(crate) fn begin_operation(
        &mut self,
        operation: SyncOperation,
        trigger: SyncTrigger,
        message: String,
        cx: &mut Context<Self>,
    ) {
        self.state.begin(operation, trigger, message);
        cx.notify();
    }

    pub(crate) fn record_history(&self, record: SyncHistoryRecord<'_>) -> SyncHistoryEntry {
        let at_ms = chrono::Utc::now().timestamp_millis();
        let entry = SyncHistoryEntry {
            id: format!(
                "{at_ms}-{}-{}",
                record.operation.key(),
                record.trigger.key()
            ),
            at_ms,
            operation: record.operation,
            trigger: record.trigger,
            status: record.status,
            provider: record.provider.to_string(),
            duration_ms: record.duration_ms,
            sessions: record.sessions,
            folders: record.folders,
            bytes: record.bytes,
            message: record.message,
        };
        if let Some(writer) = &self.history_writer {
            writer.append(entry.clone());
        }
        entry
    }

    pub(crate) fn finish_success(&mut self, record: SyncHistoryRecord<'_>, cx: &mut Context<Self>) {
        let entry = self.record_history(record);
        self.state.finish(entry);
        cx.notify();
    }

    pub(crate) fn finish_failure(
        &mut self,
        operation: SyncOperation,
        trigger: SyncTrigger,
        provider: &str,
        message: String,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        let entry = self.record_history(SyncHistoryRecord {
            operation,
            trigger,
            status: SyncHistoryStatus::Failed,
            provider,
            duration_ms: 0,
            sessions: 0,
            folders: 0,
            bytes: 0,
            message,
        });
        self.state.finish(entry);
        cx.notify();
        self.state.message.clone()
    }

    pub(crate) fn apply_check_result(
        &mut self,
        report: SyncCheckReport,
        record: SyncHistoryRecord<'_>,
        cx: &mut Context<Self>,
    ) {
        self.state.apply_check(report);
        self.finish_success(record, cx);
    }

    pub(crate) fn finish_pull_apply(
        &mut self,
        catalog_revision: u64,
        message: String,
        cx: &mut Context<Self>,
    ) {
        self.state.scheduler.mark_catalog_synced(catalog_revision);
        self.state.active_operation = None;
        self.state.active_trigger = None;
        self.state.status = SyncHistoryStatus::Success;
        self.state.message = Some(message);
        self.pull_task = None;
        cx.notify();
    }
}
