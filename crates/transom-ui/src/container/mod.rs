//! 容器引擎管理工作区：状态、命令与事件路由。

mod actions;
mod compose;
mod detail;
mod engines;
mod events;
mod files_actions;
mod form;
mod lifecycle;
mod list;
mod logs_actions;
mod rename;
mod sidebar;
mod stats;
mod stats_actions;
mod terminal;
mod toast;
mod types;
mod view;

pub(crate) use types::{
    ContainerBatchAction, ContainerConfirm, ContainerDetailTab, ContainerDiscovery,
    ContainerFilesState, ContainerSection, ContainerTerminalState, ContainerUiState,
    ContainerWorkspaceState, ContainerWorkspaceStatus, RowAction, RowTarget,
};

use crate::app::{AppShell, AppState, TabContent, TabStatus, TerminalTab, app_settings};
use crate::container::toast::{command_toast_label, toast_failed, toast_succeeded};
use crate::container::types::{
    ContainerCreateDialog, ContainerPruneTarget, STATS_HISTORY_LIMIT, arm_or_fire, row_key,
};
use crate::palette::localized_error;
use gpui_kit::component::input::InputState;
use gpui_kit::*;
use std::sync::Arc;
use transom_core::{
    AppError, Connection, ContainerAppEvent, ContainerCommand, ContainerConnection,
    ContainerEngineProfile, ContainerLifecycleAction, ContainerNetworkSpec, ContainerRequestId,
    Session, TabId, TerminalIo, TerminalSize,
};
use transom_terminal::{KeyFilter, TerminalEvent, TerminalView};

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
