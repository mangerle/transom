//! 对比查看器实体：持有单次对比的全部运行态与写回通道。
//!
//! 设计原理（Why）：对比面板是窗口级模态，但状态（两侧文本、差异、
//! 在途操作、确认态）与写回命令只属于这次对比；放进 `AppState` 会让
//! 外壳承担本属于面板的异步状态机。实体化后，后台计算、本地读写与
//! 远端命令投递都在实体内闭环，外壳只负责打开、关闭与远端事件转发。
//!
//! 代价与局限：远端写回成功后仍需外壳更新 SFTP 列表条目，这里通过
//! `DiffViewerEvent::RemoteEntryUpdated` 把结果交回外壳，避免实体反向
//! 读取工作区。

use crate::diff_viewer::DiffViewerState;
use crate::editor::EditorPanels;
use gpui_kit::*;
use transom_core::{AppCommand, TabId};

/// 对比查看器对外事件：状态变化与远端列表更新。
#[derive(Clone, Debug)]
pub(crate) enum DiffViewerEvent {
    /// 实体状态已变化，外壳需要重绘模态弹窗。
    Changed,
    /// 远端写回后文件大小/时间变化，外壳据此更新 SFTP 列表。
    RemoteEntryUpdated {
        tab_id: TabId,
        path: String,
        size: u64,
        modified: Option<u32>,
    },
}

pub(crate) struct DiffViewerView {
    pub(crate) state: DiffViewerState,
    pub(crate) command_tx: flume::Sender<AppCommand>,
    pub(crate) editors: Entity<EditorPanels>,
}

impl EventEmitter<DiffViewerEvent> for DiffViewerView {}

impl DiffViewerView {
    pub(crate) fn new(
        state: DiffViewerState,
        command_tx: flume::Sender<AppCommand>,
        editors: Entity<EditorPanels>,
    ) -> Self {
        Self {
            state,
            command_tx,
            editors,
        }
    }

    pub(crate) fn notify_changed(&self, cx: &mut Context<Self>) {
        cx.emit(DiffViewerEvent::Changed);
        cx.notify();
    }
}
