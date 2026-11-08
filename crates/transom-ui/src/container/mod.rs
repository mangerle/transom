//! 容器引擎管理工作区：状态、命令与事件路由。

mod actions;
mod build_dialog;
mod commit;
mod compose;
mod compose_templates;
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
mod sidebar;
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

pub(crate) use entity::ContainerWorkspace;
pub(crate) use event::ContainerWorkspaceEvent;
pub(crate) use pull_drawer::render_pull_drawer;
pub(crate) use pull_manager::PullTaskState;
pub(crate) use types::{
    ContainerBuildDialog, ContainerCommitState, ContainerConfirm, ContainerDetailTab,
    ContainerDiscovery, ContainerFilePrompt, ContainerFilesState, ContainerHubDialog,
    ContainerPushDialog, ContainerSection, ContainerTerminalState, ContainerUiState,
    ContainerWorkspaceState, ContainerWorkspaceStatus, RowAction, RowTarget,
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
    /// 迁移期辅助：只读访问某个标签页的容器工作区状态。
    ///
    /// 这是实体化迁移期的临时桥，最终目标是删除并按领域方法迁入
    /// [`ContainerWorkspace`]；新代码应优先取实体句柄后自行 `read`。
    pub(crate) fn with_container<R>(
        &self,
        tab_id: TabId,
        cx: &App,
        f: impl FnOnce(&ContainerWorkspaceState) -> R,
    ) -> Option<R> {
        let workspace = self.state.container_workspace(tab_id)?;
        Some(f(&workspace.read(cx).state))
    }

    /// 迁移期辅助：可变访问某个标签页的容器工作区状态。
    ///
    /// 闭包内只允许修改状态，禁止调用任何外壳方法（外壳此时已被可变借用）；
    /// 需要外壳动作时，把数据通过返回值带出闭包，闭包结束后再调用。
    /// 这是实体化迁移期的临时桥，最终目标是删除并按领域方法迁入
    /// [`ContainerWorkspace`]。
    pub(crate) fn with_container_mut<R>(
        &mut self,
        tab_id: TabId,
        cx: &mut Context<AppShell>,
        f: impl FnOnce(&mut ContainerWorkspaceState) -> R,
    ) -> Option<R> {
        let workspace = self.state.container_workspace(tab_id)?;
        Some(workspace.update(cx, |workspace, _| f(&mut workspace.state)))
    }

    /// 按请求 id 取走它所属的容器标签页。
    ///
    /// 请求登记在各标签页的容器实体内部，事件只带 `request_id`，因此必须逐个
    /// 标签反查。命中即 `take`，保证同一条结果只被处理一次；标签关闭后实体随之
    /// 释放，这里查不到任何标签页，迟到结果被自然丢弃。
    pub(crate) fn take_container_request(
        &mut self,
        request_id: ContainerRequestId,
        cx: &mut Context<AppShell>,
    ) -> Option<TabId> {
        for tab in self.state.workspace.tabs.iter() {
            let Some(workspace) = tab.containers() else {
                continue;
            };
            if !workspace.read(cx).has_request(request_id) {
                continue;
            }
            if workspace.update(cx, |workspace, _| workspace.take_request(request_id)) {
                return Some(tab.id);
            }
        }
        None
    }

    /// 只读查找请求所属的容器标签页；不消费登记。
    ///
    /// 用于同一请求会产生多条结果的事件（日志块、统计帧、传输进度）：这些事件
    /// 必须反复定位标签页，但不能取走登记，最终收尾事件才调用
    /// [`Self::take_container_request`]。
    pub(crate) fn container_request_tab(
        &self,
        request_id: ContainerRequestId,
        cx: &App,
    ) -> Option<TabId> {
        self.state.workspace.tabs.iter().find_map(|tab| {
            let workspace = tab.containers()?;
            workspace.read(cx).has_request(request_id).then_some(tab.id)
        })
    }

    /// 只读取回请求的动作描述；必须在 `take_container_request` 之前调用。
    ///
    /// 结果事件需要「启动 web-1 成功」这类文案，而取走登记后描述就不在了，
    /// 所以收尾路径的顺序固定为「先取 label，再取 tab」。
    pub(crate) fn container_request_label(
        &self,
        request_id: ContainerRequestId,
        cx: &App,
    ) -> Option<String> {
        self.state.workspace.tabs.iter().find_map(|tab| {
            tab.containers()
                .and_then(|workspace| workspace.read(cx).request_label(request_id))
        })
    }

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
