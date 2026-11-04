//! 容器引擎管理工作区：状态、命令与事件路由。

mod actions;
mod commit;
mod compose;
mod detail;
mod engines;
mod events;
mod export;
mod files_actions;
mod form;
mod lifecycle;
mod list;
mod logs_actions;
mod network_connect;
mod rename;
mod sidebar;
mod stats;
mod stats_actions;
mod tag;
mod terminal;
mod toast;
mod types;
mod update;
mod view;

pub(crate) use types::{
    ContainerConfirm, ContainerDetailTab, ContainerDiscovery, ContainerFilesState,
    ContainerSection, ContainerTerminalState, ContainerUiState, ContainerWorkspaceState,
    ContainerWorkspaceStatus, RowAction, RowTarget,
};

use crate::app::{AppShell, AppState, TabContent, TabStatus, TerminalTab, app_settings};
use crate::container::toast::{command_toast_label, toast_failed, toast_succeeded};
use crate::container::types::{
    ContainerCreateDialog, ContainerPruneTarget, STATS_HISTORY_LIMIT, row_key,
};
use crate::palette::localized_error;
use gpui_kit::component::input::InputState;
use gpui_kit::*;
use std::sync::Arc;
use transom_core::{
    AppError, Connection, ContainerAppEvent, ContainerCommand, ContainerConnection,
    ContainerEngineProfile, ContainerLifecycleAction, ContainerNetworkSpec, ContainerRequestId,
    DividerWidth, Session, TabId, TerminalIo, TerminalSize,
};
use transom_terminal::{KeyFilter, TerminalEvent, TerminalView};

/// 容器列表工具条与详情头部的统一高度。
///
/// 两块内容位于同一水平带；如果由各自内容高度决定，右侧底边会随按钮数量或
/// 文字换行而上下漂移。固定高度让两栏的分割线始终落在同一条水平线上。
pub(crate) const CONTENT_HEADER_HEIGHT: f32 = 3.0;

/// 把分割线档位换算为像素值，供需要精确覆盖父级边框的强调线使用。
pub(crate) fn divider_width_px(width: DividerWidth) -> f32 {
    match width {
        DividerWidth::Hairline | DividerWidth::Unknown => 1.0,
        DividerWidth::Medium => 2.0,
        DividerWidth::Thick => 3.0,
    }
}

impl AppState {
    /// 按标签取容器工作区状态；不是容器标签时返回 `None`。
    pub(crate) fn container_workspace(&self, tab_id: TabId) -> Option<&ContainerWorkspaceState> {
        match &self.workspace.tab(tab_id)?.content {
            TabContent::Containers(state) => Some(state),
            _ => None,
        }
    }

    /// 按标签取容器工作区的可变引用；不是容器标签时返回 `None`。
    pub(crate) fn container_workspace_mut(
        &mut self,
        tab_id: TabId,
    ) -> Option<&mut ContainerWorkspaceState> {
        match &mut self.workspace.tab_mut(tab_id)?.content {
            TabContent::Containers(state) => Some(state.as_mut()),
            _ => None,
        }
    }

    /// 汇总每个引擎连接当前的工作区状态，供侧栏行首的状态点使用。
    pub(crate) fn container_profile_states(
        &self,
    ) -> std::collections::HashMap<transom_core::EngineProfileId, ContainerWorkspaceStatus> {
        let mut states = std::collections::HashMap::default();
        for tab in self.workspace.tabs.iter() {
            let Connection::ContainerEngine(profile) = &tab.session.connection else {
                continue;
            };
            let TabContent::Containers(workspace) = &tab.content else {
                continue;
            };
            states.insert(profile.id, workspace.status.clone());
        }
        states
    }
}
