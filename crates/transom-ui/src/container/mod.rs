//! 容器引擎管理工作区：状态、命令与事件路由。

mod actions;
mod build_dialog;
mod commit;
mod compose;
mod compose_templates;
mod create_dialog_view;
mod detail;
mod engines;
mod entity;
mod event;
mod events;
mod export;
mod files_actions;
mod form;
mod hub;
mod lifecycle;
mod list;
mod logs_actions;
mod modals;
mod network_connect;
mod pods;
mod port_forward;
mod pull_drawer;
mod pull_manager;
mod push_dialog;
mod quadlet;
mod rename;
mod requests;
mod sidebar;
mod sidebar_events;
mod stats;
mod stats_actions;
mod system_df_dialog;
mod system_df_view;
mod tag;
mod terminal;
mod toast;
mod types;
mod update;
mod view;

pub(crate) use build_dialog::render_container_build_dialog_body;
pub(crate) use create_dialog_view::{ContainerCreateDialogEvent, ContainerCreateDialogView};
pub(crate) use entity::ContainerWorkspace;
pub(crate) use event::ContainerWorkspaceEvent;
pub(crate) use form::{
    ContainerFormEvent, ContainerFormFolders, ContainerFormView, ContainerRunFormEvent,
    ContainerRunFormView,
};
pub(crate) use hub::render_container_hub_dialog_body;
pub(crate) use modals::{
    render_container_commit_dialog_body, render_container_confirm_dialog_body,
    render_container_export_config_dialog_body, render_container_file_new_dir_dialog_body,
    render_container_file_rename_dialog_body, render_container_forward_dialog_body,
    render_container_network_connect_dialog_body, render_container_pods_dialog_body,
    render_container_quadlet_dialog_body, render_container_rename_dialog_body,
    render_container_tag_dialog_body, render_container_update_dialog_body,
};
pub(crate) use pull_drawer::render_pull_drawer;
pub(crate) use pull_manager::PullTaskState;
pub(crate) use push_dialog::render_container_push_dialog_body;
pub(crate) use sidebar::{
    ContainerSidebarDrag, ContainerSidebarEvent, ContainerSidebarParams, ContainerSidebarTarget,
    ContainerSidebarView, render_container_sidebar, render_engine_folder_form_body,
};
pub(crate) use system_df_dialog::render_container_system_df_dialog_body;
pub(crate) use types::{
    ContainerBuildDialog, ContainerCommitState, ContainerConfirm, ContainerDetailTab,
    ContainerDiscovery, ContainerFilePrompt, ContainerFilesState, ContainerHubDialog,
    ContainerPushDialog, ContainerSection, ContainerTerminalState, ContainerUiState,
    ContainerWorkspaceState, ContainerWorkspaceStatus, RowAction, RowTarget,
};
pub(crate) use view::{
    render_container_connection_state, render_container_list_content, render_container_tab_body,
};

use crate::app::{AppShell, AppState, TabContent, TabStatus, TerminalTab, app_settings};
use crate::container::toast::{toast_failed, toast_succeeded};
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
    Session, TabId, TerminalIo, TerminalSize,
};
use transom_terminal::{KeyFilter, TerminalEvent, TerminalView};

/// 容器列表工具条与详情头部的统一高度。
///
/// 两块内容位于同一水平带；如果由各自内容高度决定，右侧底边会随按钮数量或
/// 文字换行而上下漂移。固定高度让两栏的分割线始终落在同一条水平线上。
pub(crate) const CONTENT_HEADER_HEIGHT: f32 = crate::theme::tokens::size::LARGE_HEADER_HEIGHT;

impl AppState {
    /// 按标签取容器工作区实体句柄；不是容器标签时返回 `None`。
    ///
    /// 返回的是句柄而不是引用：调用方自行决定 `read` 还是 `update`，避免同一
    /// 标签页在渲染期与动作期各持一份状态。
    pub(crate) fn container_workspace(&self, tab_id: TabId) -> Option<Entity<ContainerWorkspace>> {
        match &self.workspace.tab(tab_id)?.content {
            TabContent::Containers(workspace) => Some(workspace.clone()),
            _ => None,
        }
    }

    /// 当前活动标签对应的引擎连接 id；不是容器标签时返回 `None`。
    ///
    /// 纯状态查询，不依赖 GPUI 上下文，因此放在 [`AppState`] 上。
    pub(crate) fn active_engine_profile_id(&self) -> Option<transom_core::EngineProfileId> {
        let tab = self
            .workspace
            .active_tab
            .and_then(|tab_id| self.workspace.tab(tab_id))?;
        match &tab.session.connection {
            Connection::ContainerEngine(profile) => Some(profile.id),
            _ => None,
        }
    }

    /// 汇总每个引擎连接当前的工作区状态，供侧栏行首的状态点使用。
    ///
    /// 状态在实体内部，汇总时必须逐个 `read`；渲染期只读，不触发任何写动作。
    pub(crate) fn container_profile_states(
        &self,
        cx: &App,
    ) -> std::collections::HashMap<transom_core::EngineProfileId, ContainerWorkspaceStatus> {
        let mut states = std::collections::HashMap::default();
        for tab in self.workspace.tabs.iter() {
            let Connection::ContainerEngine(profile) = &tab.session.connection else {
                continue;
            };
            let Some(workspace) = tab.containers() else {
                continue;
            };
            states.insert(profile.id, workspace.read(cx).state.status.clone());
        }
        states
    }
}

impl AppShell {
    /// 寻找当前需要响应容器弹窗的工作区（优先当前活动标签，回退至首个包含
    /// 该弹窗状态的标签）。
    ///
    /// 返回实体句柄而不是引用：谓词在 `read` 守卫内执行，返回前即释放，
    /// 调用方随后自行 `update`，不会在写闭包里再反查同一实体。
    pub(crate) fn find_container_modal_tab<F>(
        &self,
        cx: &App,
        predicate: F,
    ) -> Option<(TabId, Entity<ContainerWorkspace>)>
    where
        F: Fn(&ContainerWorkspaceState) -> bool,
    {
        let search = |tab_id: TabId, workspace: Entity<ContainerWorkspace>| {
            predicate(&workspace.read(cx).state).then_some((tab_id, workspace))
        };
        if let Some(tab_id) = self.state.workspace.active_tab
            && let Some(workspace) = self.state.container_workspace(tab_id)
            && let Some(found) = search(tab_id, workspace)
        {
            return Some(found);
        }
        for tab in self.state.workspace.tabs.iter() {
            let Some(workspace) = tab.containers() else {
                continue;
            };
            if let Some(found) = search(tab.id, workspace) {
                return Some(found);
            }
        }
        None
    }
}
